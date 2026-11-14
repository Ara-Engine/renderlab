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
// ---- Lean shading: a module specialized for frames that use no decals and no media ----
/// `RENDERLAB_LEAN=0` keeps the full module on every frame (A/B); on otherwise.
pub(crate) fn lean_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_LEAN").map_or(true, |v| v != "0"))
}
/// Which engine inputs a frame uses that a traced shading module must keep. A frame using none
/// of them shades with `lean_variant`'s module, byte-identical in its picture by construction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FrameInputs {
    /// Any decal projects this frame (a non-empty decal table).
    pub decals: bool,
    /// Aerial perspective, height fog or volumetric fog (the atmosphere's media flags).
    pub media: bool,
}
impl FrameInputs {
    /// Whether the lean module shades this frame exactly as the full one does. One variant,
    /// not a lattice: pane transmission stays in it (glass under a sun is common, and its
    /// cut was not where the time was).
    pub fn lean(self) -> bool {
        !self.decals && !self.media
    }
}
/// Each lean edit: the engine function it opens, and the early return that is exactly what the
/// function returns when its input is unused (an empty decal table leaves the receiver's
/// `decal_none`; no media flag leaves the colour). The dead body
/// after the return is what the compiler drops, which is the point: idle feature code still
/// costs registers (M5: decals 0.25 ms of shading with none in the scene).
const LEAN_EDITS: [(&str, &str); 2] = [
    (
        "fn decal_prepare(input:SurfaceInput)->SurfaceInput {\n",
        "    if true { return input; }\n",
    ),
    (
        "fn atmosphere_surface(c:vec3<f32>, p:vec3<f32>) -> vec3<f32> {\n",
        "    if true { return c; }\n",
    ),
];
/// `composed` (a traced shading module) for frames whose `FrameInputs::lean` holds: each lean
/// edit applied where its function is composed, the program's range moved past them.
pub(crate) fn lean_variant(mut composed: Composed) -> Composed {
    for (head, early) in LEAN_EDITS {
        if composed.source.matches(head).count() != 1 {
            continue;
        }
        let at = composed.source.find(head).map(|i| i + head.len());
        let Some(at) = at else { continue };
        assert!(
            at <= composed.program.start || at >= composed.program.end,
            "a lean edit inside the program"
        );
        composed.source.insert_str(at, early);
        if at <= composed.program.start {
            composed.program = composed.program.start + early.len()..composed.program.end + early.len();
        }
    }
    composed
}
// ---- end of lean shading ----
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
/// `RENDERLAB_RL_PASS` / `RENDERLAB_RL_DEFERRED`: "1" forces a switch on (RT included, for A/B),
/// "0" forces it off; unset follows the path's default. Read once per process.
fn env_switch(name: &str) -> Option<bool> {
    match std::env::var(name).as_deref() {
        Ok("1") => Some(true),
        Ok("0") => Some(false),
        _ => None,
    }
}
pub(crate) fn rl_pass_env() -> Option<bool> {
    static ON: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *ON.get_or_init(|| env_switch("RENDERLAB_RL_PASS"))
}
pub(crate) fn rl_deferred_env() -> Option<bool> {
    static ON: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        if rl_deferred_diag() {
            Some(true)
        } else {
            env_switch("RENDERLAB_RL_DEFERRED")
        }
    })
}
/// `RENDERLAB_RL_DEFERRED=diag`: deferred reflections on, plus the untraced specular output
/// (binding 63: the program's untraced specular, and the inline traced luminance in alpha) that
/// identity tests read (the port's RL#79 fixture). The composite never reads it, so without
/// `diag` no module declares or writes it and no backend allocates it.
pub(crate) fn rl_deferred_diag() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_RL_DEFERRED").as_deref() == Ok("diag"))
}
/// The mark a renderer whose device runs the compacted reflection pass by default (the non-RT
/// path where the adapter holds it, once reflection_pass.rs `NON_RT_DEFAULT` is on; off for now,
/// see `default_on`) appends to its own traced
/// shading heads. Composition reads it from the head, so the default belongs to that renderer:
/// heads composed elsewhere (the Ara port, tests) carry no mark and keep the inline path.
pub(crate) const PASS_HEAD_MARK: &str =
    "\n// renderlab: this renderer runs the compacted reflection pass and its deferred composite\n";
/// Whether a module composed on `source` (a head, or a module composed on one) runs the pass:
/// RENDERLAB_RL_PASS when set, otherwise its renderer's mark.
pub(crate) fn rl_pass_for(source: &str) -> bool {
    rl_pass_env().unwrap_or(source.contains(PASS_HEAD_MARK))
}
/// Whether reporting programs composed on `source` hand their glossy reflections to the deferred
/// composite: RENDERLAB_RL_DEFERRED when set, otherwise its renderer's mark.
pub(crate) fn rl_deferred_for(source: &str) -> bool {
    rl_deferred_env().unwrap_or(source.contains(PASS_HEAD_MARK))
}
/// `rl_trace`'s module on a traced shading `head`: the standard program's traced composition
/// (the reflection block's helpers, with its stubs) and the trace entries.
pub(crate) fn rl_trace_source(head: &str) -> String {
    let composed = compose(head, crate::STANDARD_PROGRAM, Stage::Traced, false);
    rl_pass_switches(format!("{}\n{REFLECTION_TRACE}", composed.source))
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
/// stubs, each while its switch is on (wgpu primary shading modules only). Unset, the module's
/// text is the default's (reflections.wgsl as composed).
pub(crate) fn with_reflection_history(composed: Composed) -> Composed {
    let composed = if rl_pass_for(&composed.source) {
        pass_block(composed)
    } else {
        composed
    };
    let composed = if inline_stub() {
        without_inline_strata(composed)
    } else {
        composed
    };
    with_history_block(composed)
}
/// Measurement only (`RENDERLAB_RL_INLINE_STUB=1`, branch t3/rtvis-stub): the primary shading
/// modules lose the inline strata loop (its ray casts and hit shading), so receivers the pass does
/// not take (mirrors, floored, overflow) reflect nothing. The picture is wrong; the shading
/// kernel's time is the upper bound of moving every traced receiver into the pass.
fn inline_stub() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_RL_INLINE_STUB").is_ok_and(|v| v == "1"))
}
/// `composed` with reflections.wgsl's strata loop removed from the engine text before the program.
fn without_inline_strata(composed: Composed) -> Composed {
    const BEGIN: &str = "    for(var attempt=0u;attempt<2u;attempt+=1u) {\n";
    const END: &str = "    rl_reflection_hit=false;\n";
    let Composed { source, program } = composed;
    let (engine, rest) = source.split_at(program.start);
    let Some(begin) = engine.find(BEGIN) else {
        return Composed { source, program };
    };
    let end = begin + engine[begin..].find(END).expect("inline stub seam");
    let engine = format!("{}{}", &engine[..begin], &engine[end..]);
    let start = engine.len();
    Composed {
        source: format!("{engine}{rest}"),
        program: start..start + program.len(),
    }
}
/// reflections.wgsl's lobe store, and its replacement under the pass: a reporting program's
/// glossy receiver that fits the room (`rl_room_reserve`) appends its lobe to the glossy-pixel
/// list and leaves its estimate to the composite (rgb 0, the traced fraction kept); every other
/// program, and a receiver that does not fit, keeps the inline path (no entry, no zero).
const PASS_GATE: [&str; 2] = [
    "    if RL_PASS && !mirror {rl_lobe_store(pixel,origin,n,v,side,alpha,width,pixel_angle+curvature);}\n",
    "    if RL_PASS && !mirror && RL_DEFERRED_PROGRAM && rl_composite_floor(traced,count) && rl_any_stratum(count,radial,rotation,basis,local_v,v,alpha,side) && rl_room_reserve(count,traced,lobe.weight) {\n        rl_lobe_store(pixel,origin,n,v,side,alpha,width,pixel_angle+curvature,count,receiver.epsilon);\n        rl_specular=vec4<f32>(0.0,0.0,0.0,traced);return;\n    }\n",
];
const DEFERRED_PROGRAM_FLAG: [&str; 2] = [
    "const RL_DEFERRED_PROGRAM:bool=false;",
    "const RL_DEFERRED_PROGRAM:bool=true;",
];
/// `composed` with the reflection pass's lobe block (shaders/reflections_lobe.wgsl) and gate,
/// whatever the process switches: `RL_DEFERRED_PROGRAM` is on for a deferred_variant module.
/// A module without the reflection block is returned as it is.
pub(crate) fn pass_block(composed: Composed) -> Composed {
    if !composed.source.contains(RL_PASS_STUBS[0]) {
        return composed;
    }
    let deferred = composed.source.contains(DEFERRED_ENTRY_MARK);
    let mut lobe = if deferred {
        REFLECTION_LOBE.replacen(DEFERRED_PROGRAM_FLAG[0], DEFERRED_PROGRAM_FLAG[1], 1)
    } else {
        REFLECTION_LOBE.to_owned()
    };
    // Measurement runs count the room's low-weight receivers (reflections_lobe.wgsl).
    if ["RENDERLAB_RL_PROBE", "RENDERLAB_RL_ROOM_LOG"]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        lobe = lobe.replacen(
            "const RL_ROOM_HISTOGRAM:bool=false;",
            "const RL_ROOM_HISTOGRAM:bool=true;",
            1,
        );
    }
    let composed = replace_block(composed, RL_PASS_STUBS, &lobe);
    let Composed { source, program } = composed;
    let (engine, rest) = source.split_at(program.start);
    assert_eq!(engine.matches(PASS_GATE[0]).count(), 1, "reflection pass gate seam");
    let engine = engine.replacen(PASS_GATE[0], PASS_GATE[1], 1);
    let start = engine.len();
    Composed {
        source: format!("{engine}{rest}"),
        program: start..start + program.len(),
    }
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
/// `strata=N` caps a glossy pixel's strata at N (RL_GLOSSY_MAX; `strata=16` is RQ1's original cap).
/// Two act on every pixel, camera receivers included, to split lights and shadows between
/// primary pixels and reflection hits: `alllights` (every light zero incidence) and
/// `allshadows` (every light fully visible). They change the picture; unset, nothing changes.
fn rl_hit_switches(source: String) -> String {
    static LIST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(|| std::env::var("RENDERLAB_RL_HIT").unwrap_or_default());
    let mut source = source;
    for switch in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(n) = switch.strip_prefix("strata=") {
            // Whatever the shipped default is (shaders/reflections.wgsl), `strata=16` restores
            // RQ1's cap for A/Bs.
            let key = "const RL_GLOSSY_MAX:u32=";
            assert_eq!(source.matches(key).count(), 1, "RENDERLAB_RL_HIT strata");
            let at = source.find(key).unwrap();
            let end = at + source[at..].find(';').unwrap() + 1;
            source.replace_range(at..end, &format!("{key}{n}u;"));
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
/// The pass's own modules under the same list (measurement only, RT speck bisect): `missblack`
/// stores black for `rl_trace_pixels`' misses instead of the sky, `hitblack` stores black for
/// `reflection_hits`' hits, `lone` makes the composite apply the lone-stratum bound whenever two
/// strata are valid. Each applies where its seam is (trace, hits, composite); unset, nothing changes.
pub(crate) fn rl_pass_switches(source: String) -> String {
    static LIST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(|| std::env::var("RENDERLAB_RL_HIT").unwrap_or_default());
    let mut source = source;
    for switch in list.split(',').map(str::trim) {
        let Some((_, from, to)) = RL_PASS_SEAMS.iter().find(|(name, ..)| *name == switch) else {
            continue;
        };
        assert!(source.matches(from).count() <= 1, "RENDERLAB_RL_HIT pass seam {switch}");
        source = source.replacen(from, to, 1);
    }
    source
}
/// `rl_pass_switches`' seams: switch, text, replacement.
pub(crate) const RL_PASS_SEAMS: [(&str, &str, &str); 3] = [
    (
        "missblack",
        "textureStore(rl_samples,at,rl_f16_nearest(vec4<f32>(rl_sky(d,cone_alpha),1.0)));",
        "textureStore(rl_samples,at,vec4<f32>(0.0,0.0,0.0,1.0));",
    ),
    (
        "hitblack",
        "    let radiance=select(vec3<f32>(0.0),rl_finite(shade(hs,h.input)),h.visible);\n    textureStore(rl_hit_samples_out",
        "    let radiance=vec3<f32>(0.0);\n    textureStore(rl_hit_samples_out",
    ),
    (
        "lone",
        "if lone>0.0 && valid>1u && f32(count)>=rl_composite_frame.y {",
        "if lone>0.0 && valid>1u {",
    ),
];

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
const VIEW_STUB: &str = "fn tr1_view_hint(s:Surface,input:SurfaceInput,ray:PrimaryRay)->f32 { return 1.0; }\n";
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
        "    rayQueryInitialize(&query,RL_SCENE,RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT|RAY_FLAG_FORCE_OPAQUE,SHADOW_CASTER_MASK,receiver.epsilon,tmax,origin,l));\n    while rayQueryProceed(&query) {}\n    return select(1.0,0.0,rayQueryGetCommittedIntersection(&query).kind!=0u);\n",
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
    let composed = compose_stage(head, program, stage, parameters, false);
    match (stage, light_bound()) {
        (Stage::Traced | Stage::TracedStreamed, Some(bound)) => with_light_bound(composed, bound),
        _ => composed,
    }
}
/// `RENDERLAB_LIGHT_BOUND=black|sum` (off by default; a bounded picture change, not exact):
/// the engine's `light_visibility_within(input, light, unshadowed, sum)` (program_api.wgsl)
/// skips a local shadowed light's shadow ray while its unshadowed exposed contribution, added
/// to those already skipped at this receiver, stays under `RENDERLAB_LIGHT_BOUND_CODES` display
/// codes (default 0.5). The bound goes through Ara's display transform: the Filmic (Narkowicz
/// ACES fit) curve and sRGB. `black` uses the curve's steepest slope (order-free); `sum` a
/// pow-free upper bound of the slope at or above the receiver's running sum (order-dependent).
/// A skipped light counts as unshadowed (`RENDERLAB_LIGHT_BOUND_SKIP=unshadowed`, default) or
/// dark (`drop`): either way its error is at most its contribution. Traced stages only; never
/// at reflection hits. A program that calls `light_visibility` is never bounded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LightBound {
    /// The `sum` mode (else `black`).
    pub sum: bool,
    /// The bound in display codes.
    pub codes: f32,
    /// Skipped lights are dropped instead of added unshadowed.
    pub drop: bool,
}
pub(crate) fn light_bound() -> Option<LightBound> {
    static BOUND: std::sync::OnceLock<Option<LightBound>> = std::sync::OnceLock::new();
    *BOUND.get_or_init(|| {
        let sum = match std::env::var("RENDERLAB_LIGHT_BOUND").ok()?.as_str() {
            "black" => false,
            "sum" => true,
            other => {
                eprintln!("RENDERLAB_LIGHT_BOUND: `{other}` is not black or sum; off");
                return None;
            }
        };
        let codes = std::env::var("RENDERLAB_LIGHT_BOUND_CODES")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|c| *c > 0.0)
            .unwrap_or(0.5);
        let drop = std::env::var("RENDERLAB_LIGHT_BOUND_SKIP").is_ok_and(|v| v == "drop");
        Some(LightBound { sum, codes, drop })
    })
}
/// The program API's stub of the bound (shaders/program_api.wgsl), which `with_light_bound`
/// replaces on traced stages.
const LB_STUB: [&str; 2] = [
    "// ---- light bound (program.rs with_light_bound replaces this block on traced stages) ----\n",
    "// ---- end of light bound ----\n",
];
/// Where each receiver's skipped total starts over (shaders/light_cluster_api.wgsl).
const LB_RESET: &str = "fn rl_lights_prepare(world:vec3<f32>) {\n";
/// `composed` with the engine's light bound (`light_bound`) behind `light_visibility_within`:
/// the API's stub replaced, the skipped total reset per receiver in `rl_lights_prepare`, and
/// off while a reflection hit is shaded. Unchanged without the stub or the reset.
pub(crate) fn with_light_bound(mut composed: Composed, bound: LightBound) -> Composed {
    let source = &composed.source;
    let (Some(begin), Some(reset)) = (source.find(LB_STUB[0]), source.find(LB_RESET)) else {
        return composed;
    };
    let end = source[begin..].find(LB_STUB[1]).expect("the light bound stub ends") + begin + LB_STUB[1].len();
    assert!(end <= composed.program.start && reset < composed.program.start, "the light bound precedes the program");
    // With RENDERLAB_SHADE_COUNT's counters composed in (measurement builds), words 14 and 15
    // count the local shadowed lights tested and those whose shadow ray was skipped.
    let counted = source.contains("fn rl_shade_count()");
    // Reflection hits always trace: their running sum is the hit's, not the pixel's.
    let hits = if source.contains("var<private> rl_reflection_hit:bool;") {
        "    if rl_reflection_hit { return false; }\n"
    } else {
        ""
    };
    let real = format!(
        "// ---- light bound (program.rs light_bound) ----\n\
const RL_LB_SUM:bool={sum};\n\
const RL_LB_CODES:f32={codes:?};\n\
const RL_LB_SKIPPED:f32={skipped:?};\n\
// The Filmic curve's (Narkowicz) display slope peaks at 6.38 codes/255 per unit at x=0.0087\n\
// and only falls above it; the bounds below carry a 1% margin.\n\
const RL_LB_PEAK:f32=6.443;\n\
// The contributions skipped at this receiver (reset by rl_lights_prepare).\n\
var<private> rl_lb_skipped:vec3<f32>;\n\
// An upper bound of d(sRGB(filmic(x)))/dx at or above x, per channel, without pow: the peak\n\
// below 0.0087, else min(peak, 0.2888/x, 0.1082/x^2, 0.7648/x^3), fitted with a 1% margin\n\
// (never under the slope; at most 1.6x over it). Looser only skips less.\n\
fn rl_lb_slope(x:vec3<f32>) -> vec3<f32> {{\n    let r=1.0/max(x,vec3<f32>(0.0087));\n    return min(min(vec3<f32>(RL_LB_PEAK),0.2888*r),min(0.1082*r*r,0.7648*r*r*r));\n}}\n\
fn rl_lb_skip(s:LightSample, c:vec3<f32>, sum:vec3<f32>) -> bool {{\n{hits}    if lights[s.index].kind.w==0u {{ return false; }}\n{tested}    let total=rl_lb_skipped+c;\n    var slope=vec3<f32>(RL_LB_PEAK);\n    if RL_LB_SUM {{ slope=rl_lb_slope(sum); }}\n    if all(total*slope<vec3<f32>(RL_LB_CODES/255.0)) {{ rl_lb_skipped=total;{skipped_count} return true; }}\n    return false;\n}}\n\
// ---- end of light bound ----\n",
        sum = bound.sum,
        codes = bound.codes,
        skipped = if bound.drop { 0.0f32 } else { 1.0f32 },
        tested = if counted { "    atomicAdd(&rl_shade_counts[14],1u);\n" } else { "" },
        skipped_count = if counted { " atomicAdd(&rl_shade_counts[15],1u);" } else { "" },
    );
    let mut source = composed.source.clone();
    // The reset goes in after the block (it follows it in the source, or comes first).
    let reset_line = "    rl_lb_skipped=vec3<f32>(0.0);\n";
    if reset > begin {
        source.insert_str(reset + LB_RESET.len(), reset_line);
        source.replace_range(begin..end, &real);
    } else {
        source.replace_range(begin..end, &real);
        source.insert_str(reset + LB_RESET.len(), reset_line);
    }
    let shift = source.len() - composed.source.len();
    composed.program = composed.program.start + shift..composed.program.end + shift;
    composed.source = source;
    composed
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
    let mut deferred = false;
    if hits {
        // The stage's entry as written (its decode helpers are the hits' too; its own entry
        // point goes unused), then the hit entry.
        source.push_str(entry);
        source.push_str(crate::renderer::reflection_hits::ENTRY);
    } else if traces {
        // M7-R1: the lobe is traced between the receiver's `surface` and `shade`.
        assert_eq!(entry.matches(REFLECTION_ENTRY_SEAM).count(), 1);
        source.push_str(&entry.replacen(REFLECTION_ENTRY_SEAM, REFLECTION_ENTRY, 1));
        deferred = rl_deferred_for(head) && declares(program, "const", "RL_DEFERRED");
    } else {
        source.push_str(entry);
    }
    // Every module takes the GI pass's splice, deferred ones included: the GI receiver kernel
    // (gi_receiver_pass.rs) relies on the definitions it appends.
    source = crate::renderer::primary_ray_gpu::gi_receiver_pass::shade_source(source, matches!(stage, Stage::Traced | Stage::TracedStreamed) && indirect_calls, specular);
    // Light visibility (light_vis_pass.rs) and then the deferred variant, for every module.
    let composed = crate::renderer::primary_ray_gpu::light_vis_pass::apply(
        Composed { source, program: program_range },
        matches!(stage, Stage::Traced | Stage::TracedStreamed),
    );
    if deferred {
        deferred_variant(composed)
    } else {
        composed
    }
}

// ---- Deferred reflections (the program side; U-RTVIS's composite reads them) ----
/// Whether `RENDERLAB_RL_DEFERRED` forces the deferred variant on (1 or diag), whatever the
/// renderer's default. The Mac thread's port test reads it.
#[allow(dead_code)]
pub(crate) fn rl_deferred() -> bool {
    rl_deferred_env() == Some(true)
}
/// The program API's report, which deferred modules replace with `DEFERRED_REPORT`.
const DEFERRED_REPORT_STUB: &str =
    "fn reflection_report(reflectance:vec3<f32>, untraced:vec3<f32>) {}";
const DEFERRED_API: &str = include_str!("../shaders/reflections_deferred.wgsl");
/// What a deferred_variant module's entry gains after its pixel: its outputs cleared. Its
/// presence marks the module as deferred (`pass_block`).
const DEFERRED_ENTRY_MARK: &str = "\n    rl_deferred_clear(pixel);";
/// The traced entry's pixel, where a deferred module first clears its outputs.
const DEFERRED_CLEAR_SEAM: &str =
    "let pixel=vec2<i32>(id);let record=textureLoad(primary_hit,pixel,0);";
/// A traced shading module (`compose` output) of a program that declares `const
/// RL_DEFERRED`, made to write the deferred composite's inputs: the report keeps the
/// program's reflectance and untraced specular, and the entry writes them through the pixel's
/// fog and aerial perspective, with the traced fraction (binding 62: reflectance rgb and w;
/// 63: untraced specular). Every pixel the entry visits starts cleared, so misses and culled
/// faces leave zero; under the pass the composite takes this program's glossy lobes
/// (`pass_block`). The engine text before the program and the entry after it are edited apart,
/// so the program's range follows the edits (temporal shading splits the module at the
/// program's end).
pub(crate) fn deferred_variant(composed: Composed) -> Composed {
    let Composed { source, program } = composed;
    let (engine, rest) = source.split_at(program.start);
    let (text, entry) = rest.split_at(program.len());
    for (part, seam) in [
        (engine, DEFERRED_REPORT_STUB),
        (entry, DEFERRED_CLEAR_SEAM),
        (entry, VIEW_STORE_SEAM),
    ] {
        assert_eq!(part.matches(seam).count(), 1, "deferred seam {seam}");
    }
    let engine = engine.replacen(DEFERRED_REPORT_STUB, "", 1);
    let entry = entry
        .replacen(
            DEFERRED_CLEAR_SEAM,
            &format!("{DEFERRED_CLEAR_SEAM}{DEFERRED_ENTRY_MARK}"),
            1,
        )
        .replacen(
            VIEW_STORE_SEAM,
            &format!("{VIEW_STORE_SEAM}\n    rl_deferred_store(pixel,input.world);"),
            1,
        );
    let start = engine.len();
    Composed {
        source: format!("{engine}{text}{entry}{}", deferred_api()),
        program: start..start + text.len(),
    }
}
/// DEFERRED_API as composed: with `RENDERLAB_RL_DEFERRED=diag` whole, otherwise without the
/// untraced output (binding 63's declaration and both of its stores).
fn deferred_api() -> String {
    if rl_deferred_diag() {
        return DEFERRED_API.to_owned();
    }
    let mut api = DEFERRED_API.to_owned();
    for line in [
        "@group(0) @binding(63) var rl_deferred_untraced:texture_storage_2d<rgba16float,write>;\n",
        "    textureStore(rl_deferred_untraced,pixel,vec4<f32>(0.0));\n",
        "    textureStore(rl_deferred_untraced,pixel,vec4<f32>(rl_reported[1]*t,dot(rl_specular.rgb,RL_LUMA)));\n",
    ] {
        assert_eq!(api.matches(line).count(), 1, "deferred API line {line}");
        api = api.replacen(line, "", 1);
    }
    // The reflectance only where the composite reads it (glossy receivers), never cleared.
    let full = "const RL_DEFERRED_FULL:bool=true;";
    assert_eq!(api.matches(full).count(), 1, "deferred API full switch");
    api.replacen(full, "const RL_DEFERRED_FULL:bool=false;", 1)
}

// ---- View modes (docs/design-id-output.md) ----
/// The traced entry's lit store, which a view-mode module replaces.
const VIEW_STORE_SEAM: &str = "textureStore(primary_hdr,pixel,vec4<f32>(atmosphere_surface(rl_finite(shade(surface_output,surface_input)),input.world),tr1_view_hint(surface_output,surface_input,view_ray)));";
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
        // Inline's fallback (LIST_HIT_PARAMETERS) unless RENDERLAB_RL_HIT_OWN_PROGRAM=1: the
        // receiver's program shades every hit of its pixels, with the hit's own parameter block
        // only when the hit is that program's draw, so the composite draws inline's picture.
        if !hit_own_program() {
            for (seam, fallback) in HITS_RECEIVER_PROGRAM {
                assert_eq!(composed.source.matches(seam).count(), 1, "reflection_hits seam {seam}");
                composed.source = composed.source.replacen(seam, fallback, 1);
            }
        }
    }
    // The hit entry follows the program, so the range stays.
    composed.source = rl_pass_switches(composed.source);
    composed
}
/// `RENDERLAB_RL_HIT_OWN_PROGRAM=1` (measurement, off by default): in a mixed-program frame each
/// hit is shaded by its own draw's program, with its own parameters. More correct than inline's
/// fallback (an emitter's reflection glows), but a visible change (docs/m7-gaps.md, Programs).
pub(crate) fn hit_own_program() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_RL_HIT_OWN_PROGRAM").is_ok_and(|v| v == "1"))
}
/// shaders/reflection_hits.wgsl's seams for inline's fallback in mixed frames: a record belongs to
/// the module of its receiver's program (the primary hit at the record's pixel), and the hit's
/// parameter block is used only when the hit's draw is that program's.
const HITS_RECEIVER_PROGRAM: [(&str, &str); 2] = [
    (
        "    if !rl_hits_mine(a.w-1u) {return;}\n",
        "    let rl_receiver=textureLoad(primary_hit,vec2<i32>(vec2<u32>(a.x&0xffffu,a.x>>16u)),0).x;\n    if rl_receiver==0u || !rl_hits_mine(rl_receiver-1u) {return;}\n",
    ),
    // The hit entry's own line (4 spaces), not the inline loop's (16), which the module also holds.
    (
        "\n    rl_load_parameters(h.batch);\n    let hs=surface(h.input);",
        "\n    rl_load_parameters(select(primary_placements[rl_receiver-1u]&0x1fffffffu,h.batch,rl_list.programs[h.batch>>2u][h.batch&3u]==rl_list.slot));\n    let hs=surface(h.input);",
    ),
];
/// shaders/reflection_hits.wgsl's per-program filter, and its mixed-frame replacement: the
/// hit's draw's frame slot against this dispatch's (program_traced_list.wgsl RlList, at
/// binding 38 with a dynamic offset per slot).
const HITS_MINE: &str = "fn rl_hits_mine(placement:u32)->bool { return true; }\n";
const HITS_MINE_LIST: &str = "struct RlList { slot:u32, base:u32, capacity:u32, flags:u32, programs:array<vec4<u32>,PRIMARY_BATCHES/4> };\n@group(0) @binding(38) var<uniform> rl_list:RlList;\nfn rl_hits_mine(placement:u32)->bool { let batch=primary_placements[placement]&0x1fffffffu; return rl_list.programs[batch>>2u][batch&3u]==rl_list.slot; }\n";
// ---- end of reflection hit shading ----

// ---- MP1 per-program dispatch (docs/m7-mp1.md) ----
/// The tile-list shading entry of mixed-program frames (`primary_shade_list`).
const TRACED_LIST_ENTRY: &str = include_str!("../shaders/program_traced_list.wgsl");
/// `RENDERLAB_LIST_LANES`: lanes per list-shading workgroup, 16 (one 4x4 list entry), 32 (two,
/// the default since M8-LIST-LANES-32) or 64 (four entries, one per z slice). Warps and SIMD
/// groups are 32 wide and never span workgroups, so 16 leaves half of every one idle. Each entry
/// is still shaded by exactly one slice, so the picture does not depend on it. The binning
/// kernel's indirect arguments follow (`program_lists::bin_source`).
pub(crate) fn list_entries_per_group() -> u32 {
    static PER: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *PER.get_or_init(|| match std::env::var("RENDERLAB_LIST_LANES").as_deref() {
        Ok("16") => 1,
        Err(_) | Ok("32") => 2,
        Ok("64") => 4,
        Ok(other) => {
            eprintln!("RENDERLAB_LIST_LANES: `{other}` is not 16, 32 or 64; using 32");
            2
        }
    })
}
/// The list entry with `per` list entries per workgroup (`list_entries_per_group`).
pub(crate) fn list_entry(per: u32) -> String {
    if per == 1 {
        return TRACED_LIST_ENTRY.to_owned();
    }
    let mut source = TRACED_LIST_ENTRY.to_owned();
    for (find, replace) in [
        (
            "@compute @workgroup_size(4,4) fn primary_shade_list(",
            format!("@compute @workgroup_size(4,4,{per}) fn primary_shade_list("),
        ),
        (
            "    let index=group.x+group.y*RL_LIST_ROW;\n",
            format!("    let index=(group.x+group.y*RL_LIST_ROW)*{per}u+local.z;\n"),
        ),
        // A clear block's 16 columns: one per lane of the entry's own slice.
        (
            "vec2<u32>(lane,0u)",
            "vec2<u32>(local.x+local.y*4u,0u)".to_owned(),
        ),
    ] {
        assert_eq!(source.matches(find).count(), 1, "list lanes: {find}");
        source = source.replacen(find, &replace, 1);
    }
    source
}
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
    source = format!("{edited}{rest}{}", list_entry(list_entries_per_group()));
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
/// Glass hits never report to the deferred reflection composite (their module keeps no camera
/// entry, and its glossy lobes stay inline), so under RENDERLAB_RL_DEFERRED=1 the program's
/// `RL_DEFERRED` opt-in is renamed away and `compose` leaves the module undeferred. Unset, the
/// program's text is unchanged.
pub(crate) fn glass_hit_program(program: &str, deferred: bool) -> String {
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut source = String::with_capacity(program.len());
    let mut rest = program;
    while let Some(at) = rest.find("RL_DEFERRED").filter(|_| deferred) {
        let end = at + "RL_DEFERRED".len();
        let whole = (at == 0 || !ident(rest.as_bytes()[at - 1]))
            && rest.as_bytes().get(end).is_none_or(|&b| !ident(b));
        source.push_str(&rest[..at]);
        source.push_str(if whole {
            "RL_GLASS_UNDEFERRED"
        } else {
            "RL_DEFERRED"
        });
        rest = &rest[end..];
    }
    source.push_str(rest);
    if !declares(program, "fn", "reflection_lobe") {
        source.push_str("\nfn reflection_lobe(s:Surface,input:SurfaceInput)->SpecularLobe {return SpecularLobe(input.geometric,0.0,0.0);}\n");
    }
    source.push_str("\nfn rl_glass_requires_incident(input:SurfaceInput)->vec4<f32> {return specular_incident(input);}\n");
    source
}
/// Whether a shading program's own source (as installed, before compose) calls the program
/// API's normal variance: `material_normal_variance(`, `filtered_roughness(` or
/// `filtered_brdf(`, as whole identifiers, outside `//` comments and not as their `fn`
/// definitions. A BC5 normal map has its variance layer only while an installed program does
/// (src/normal_variance_lazy.rs). Shared by the wgpu renderer and the ara-gpu port; it errs
/// towards true, which only costs a layer.
pub(crate) fn reads_normal_variance(program_source: &str) -> bool {
    const NAMES: [&str; 3] = [
        "material_normal_variance",
        "filtered_roughness",
        "filtered_brdf",
    ];
    program_source.lines().any(|line| {
        let code = line.split("//").next().unwrap_or("");
        NAMES.iter().any(|name| {
            code.match_indices(name).any(|(at, _)| {
                let before = code[..at].chars().next_back();
                let after = code[at + name.len()..].trim_start();
                let word = before.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
                word && after.starts_with('(') && !code[..at].trim_end().ends_with("fn")
            })
        })
    })
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
#[test]
fn reads_normal_variance_finds_calls_only() {
    assert!(!reads_normal_variance(include_str!(
        "../shaders/programs/standard.wgsl"
    )));
    assert!(!reads_normal_variance(include_str!(
        "../shaders/programs/toon.wgsl"
    )));
    for name in [
        "material_normal_variance",
        "filtered_roughness",
        "filtered_brdf",
    ] {
        assert!(
            reads_normal_variance(&format!("    let v={name}(input);")),
            "{name}"
        );
        assert!(
            reads_normal_variance(&format!("let v = {name} (input);")),
            "{name}"
        );
        assert!(
            !reads_normal_variance(&format!("// let v={name}(input);")),
            "{name}"
        );
        assert!(
            !reads_normal_variance(&format!("fn {name}(input:SurfaceInput) -> f32 {{")),
            "{name}"
        );
        assert!(
            !reads_normal_variance(&format!("let v=my_{name}(input);")),
            "{name}"
        );
        assert!(
            !reads_normal_variance(&format!("let {name}_scale=1.0;")),
            "{name}"
        );
    }
    // The API's own definitions: a composed module would always match a plain search.
    assert!(!reads_normal_variance(include_str!(
        "../shaders/program_api.wgsl"
    )));
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

// ---- Material pass (docs/design-material-pass.md) ----
/// `RENDERLAB_MATERIAL_PASS=1` (off by default): a program that declares the material record's
/// mapping (`material_out` / `material_in`, shaders/program_api.wgsl `MaterialRecord`) evaluates
/// `surface` once per pixel in a material pass that writes the record, and its shading reads
/// the record instead of evaluating `surface` again. Programs without the mapping keep their
/// single shading kernel. Off, nothing changes.
pub(crate) fn material_pass_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_MATERIAL_PASS").is_ok_and(|v| v == "1"))
}
/// `RENDERLAB_MATERIAL_DEBUG=base|normal|roughness|metallic` (with the pass on): the
/// record-reading shading shows that record field as the pixel's colour, unlit, so the record
/// can be checked against the scene. `receiver` writes the record's receiver against the
/// sun path's receiver for a fresh camera ray (`primary_raw_closest`, then the edge repair):
/// x the trace point's largest error (plus 1000 when the slot, `thin` or either side differs,
/// 2000 when a point the repair moved lacks the near-edge mark, 4000 when the model is 255 on a
/// mapped program's pixel or not 255 on an unmapped one's; negated, minus 1e-6, on the pixels
/// of programs without the mapping, whose debug reader replaces their own shading),
/// y the epsilon's error, z the geometric normal's angle (negative, minus 1e-6, where the
/// sun path's edge repair moved the fresh trace's point) and w minus (1e-6
/// plus) the interpolated vertex normal's angle against this pixel's decode, in radians
/// (tests/ara_backend material_pass). Misses write x 0 where word 1 holds the miss sentinel
/// (9000 where not) and w 3. Under `tracetail` the model is not checked (the trace writes no
/// material).
fn material_debug() -> Option<&'static str> {
    static FIELD: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
    *FIELD.get_or_init(|| material_debug_field(&std::env::var("RENDERLAB_MATERIAL_DEBUG").ok()?))
}
/// The debug view `name`'s pixel (`material_debug`), or None for no view.
pub(crate) fn material_debug_field(name: &str) -> Option<&'static str> {
    match name {
        "base" => Some("vec4<f32>(rl_record.base,1.0)"),
        "normal" => Some("vec4<f32>(rl_record.normal*0.5+vec3<f32>(0.5),1.0)"),
        "roughness" => Some("vec4<f32>(vec3<f32>(rl_record.roughness),1.0)"),
        "metallic" => Some("vec4<f32>(vec3<f32>(rl_record.metallic),1.0)"),
        "receiver" => Some("rl_material_receiver_error(pixel,surface_input,source_front,lit_front)"),
        _ => None,
    }
}
/// The record's group 0 binding: one rgba32uint 2D array of the frame's size, receiver first.
/// Layer 0 is word 1 and layer 1 word 2 (the receiver, `MATERIAL_RECEIVER_LAYERS`), layer 2
/// word 0 (the material, `MATERIAL_RECORD_LAYERS` in all). A receiver-only producer (the trace
/// tail) allocates and writes the first two; the material pass's writer all three.
pub(crate) const MATERIAL_RECORD_BINDING: u32 = 75;
/// The receiver's layers (words 1 and 2), first in the array.
pub(crate) const MATERIAL_RECEIVER_LAYERS: u32 = 2;
/// Every layer of the full record (the receiver's, then word 0).
pub(crate) const MATERIAL_RECORD_LAYERS: u32 = 3;
/// Which side of the record a module is: the material pass writes it, shading reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MaterialRole {
    Write,
    Read,
}
/// The entry's own evaluation of the program's surface, which the variants replace.
const MATERIAL_SEAM: &str = "    let surface_output=surface(surface_input);\n";
/// The record's packing, shared by both roles (48 bytes a pixel, binding 75's layers: word 1 is
/// layer 0, word 2 layer 1, word 0 layer 2; `MATERIAL_RECORD_BINDING`):
/// - word 0 (rgba32uint): x the shading normal (octahedral, 2 x unorm16); y the base colour's
///   red and green and z's low half its blue (3 x unorm16: 10 bits moved every pixel of a
///   flat floor by an f16 step, and turned a texture's decode difference of a hair into a
///   whole step); z's high half coverage and metallic (2 x unorm8); w the emission times this
///   frame's exposure (rgb9e5: unexposed emitters such as Glint's 40,000-nit glow pass its
///   65,408 ceiling);
/// - word 1 (rgba32uint): x the receiver's geometric normal (octahedral; `Receiver::geometric`,
///   before the sides flip it: `SurfaceInput::geometric` follows from it and the side bits); y the lighting model
///   (low 8 bits), MQ2's lobe alpha (unorm8 at bit 8) and the roughness (unorm16 at bit 16,
///   rounded up so the decoded value is never below the evaluated one: a roughness authored at
///   a threshold such as `LocalShadows::unshadowed_hits_from` stays on its side); z MQ2's view
///   share (unorm16) and MQ2's TR1 view hint (f16, high half: `tr1_view_hint`, computed here);
///   w the receiver's slot (bits 0-27), near a triangle edge (28: there the sun path's edge
///   repair may move the point, and a reader that needs its point repairs the hit itself;
///   `MATERIAL_TRACE_RECEIVER`), `thin` (29), the source-front side (30) and the lit front
///   (31). Model 255 marks a pixel whose program keeps its own `shade`;
/// - word 2 (rgba32uint): xyz the receiver's trace point (f32, bit for bit shading's
///   `Receiver::world`; the sun path's too wherever the edge repair leaves the hit);
///   w its vertex-precision epsilon as an f32 exponent (bits 0-7; 0 stands for the 1e-5 floor,
///   every other epsilon is a power of two, so both are exact) and the interpolated vertex
///   normal's direction (octahedral 2 x 12 bits, bits 8-31), which the GI and ES lookups bin
///   by. Words 1 and 2 are the receiver every later pass reads instead of decoding the hit
///   (docs/design-material-pass.md).
const MATERIAL_PACKING: &str = r#"// RENDERLAB_MATERIAL_PASS (program.rs material_pass_on): the material record's packing.
fn rl_mr_oct(n:vec3<f32>)->u32 {
    let a=n/max(abs(n.x)+abs(n.y)+abs(n.z),1e-30);
    var p=a.xy;
    if a.z<0.0 { p=(vec2<f32>(1.0)-abs(a.yx))*select(vec2<f32>(-1.0),vec2<f32>(1.0),a.xy>=vec2<f32>(0.0)); }
    return pack2x16unorm(p*0.5+vec2<f32>(0.5));
}
fn rl_mr_oct_decode(u:u32)->vec3<f32> {
    let p=unpack2x16unorm(u)*2.0-vec2<f32>(1.0);
    var n=vec3<f32>(p,1.0-abs(p.x)-abs(p.y));
    let t=max(-n.z,0.0);
    n=vec3<f32>(n.x+select(t,-t,n.x>=0.0),n.y+select(t,-t,n.y>=0.0),n.z);
    return normalize(n);
}
fn rl_mr_unorm16(v:f32)->u32 { return u32(round(clamp(v,0.0,1.0)*65535.0)); }
// Decoding multiplies by the constant's reciprocal, in both modules alike, so the upward
// rounding below checks itself against the decode the reader will run.
fn rl_mr_unorm16_decode(q:u32)->f32 { return f32(q&65535u)*(1.0/65535.0); }
fn rl_mr_unorm16_up(v:f32)->u32 {
    let c=clamp(v,0.0,1.0);
    var q=u32(ceil(c*65535.0));
    if rl_mr_unorm16_decode(q)<c { q=min(q+1u,65535u); }
    return q;
}
struct RlMaterialWords { a:vec4<u32>, b:vec4<u32>, c:vec4<u32> };
fn rl_mr_oct12(n:vec3<f32>)->u32 {
    let a=n/max(abs(n.x)+abs(n.y)+abs(n.z),1e-30);
    var p=a.xy;
    if a.z<0.0 { p=(vec2<f32>(1.0)-abs(a.yx))*select(vec2<f32>(-1.0),vec2<f32>(1.0),a.xy>=vec2<f32>(0.0)); }
    let q=vec2<u32>(round(clamp(p*0.5+vec2<f32>(0.5),vec2<f32>(0.0),vec2<f32>(1.0))*4095.0));
    return q.x|(q.y<<12u);
}
fn rl_mr_oct12_decode(u:u32)->vec3<f32> {
    let p=vec2<f32>(f32(u&4095u),f32((u>>12u)&4095u))*(2.0/4095.0)-vec2<f32>(1.0);
    var n=vec3<f32>(p,1.0-abs(p.x)-abs(p.y));
    let t=max(-n.z,0.0);
    n=vec3<f32>(n.x+select(t,-t,n.x>=0.0),n.y+select(t,-t,n.y>=0.0),n.z);
    return normalize(n);
}
fn rl_mr_epsilon(e:f32)->u32 {
    let bits=bitcast<u32>(e);
    return select(0u,(bits>>23u)&255u,(bits&0x7fffffu)==0u && e!=1e-5);
}
fn rl_mr_epsilon_decode(code:u32)->f32 { return select(1e-5,bitcast<f32>(code<<23u),code!=0u); }
// `hint` MQ2's `tr1_view_hint`; `source_front` and `lit_front` the traced entry's sides.
// Words 1 and 2 of a hit's receiver, for every producer: the material pass's writer (which
// adds word 1's material fields y and z) and a trace kernel's tail, on either engine. The
// producer supplies the receiver as `primary_receiver` builds it, the interpolated vertex
// normal (any length), both sides of the shading entry and whether the hit is near an edge
// (shading's barycentric point there may not be the sun path's: `rl_mr_near`).
struct RlReceiverWords { b:vec4<u32>, c:vec4<u32> };
fn rl_mr_receiver_words(receiver:Receiver,vertex_normal:vec3<f32>,source_front:bool,lit_front:bool,near:bool)->RlReceiverWords {
    return RlReceiverWords(
        vec4<u32>(rl_mr_oct(receiver.geometric),0u,0u,
            (receiver.slot&0x0fffffffu)|select(0u,1u<<28u,near)|select(0u,1u<<29u,receiver.thin)|select(0u,1u<<30u,source_front)|select(0u,1u<<31u,lit_front)),
        vec4<u32>(bitcast<vec3<u32>>(receiver.world),rl_mr_epsilon(receiver.epsilon)|(rl_mr_oct12(vertex_normal)<<8u)));
}
// Word 1 of a pixel without a camera hit (`rl_mr_miss`); word 2 is zero there.
const RL_MR_MISS_WORD1:vec4<u32>=vec4<u32>(0u,0u,0u,0xffffffffu);
// `near` the hit is near a triangle edge (`rl_mr_near_edge`).
fn rl_mr_pack(m:MaterialRecord,input:SurfaceInput,near:bool,hint:f32,source_front:bool,lit_front:bool)->RlMaterialWords {
    let r=rl_mr_receiver_words(input.receiver,input.normal,source_front,lit_front,near);
    let model=(m.model&255u)|(u32(round(clamp(m.view_alpha,0.0,1.0)*255.0))<<8u)|(rl_mr_unorm16_up(m.roughness)<<16u);
    let scalars=pack4x8unorm(vec4<f32>(0.0,0.0,m.alpha,m.metallic))&0xffff0000u;
    return RlMaterialWords(
        vec4<u32>(rl_mr_oct(m.normal),rl_mr_unorm16(m.base.x)|(rl_mr_unorm16(m.base.y)<<16u),rl_mr_unorm16(m.base.z)|scalars,rl_mr_rgb9e5(m.emission*lighting.exposure)),
        vec4<u32>(r.b.x,model,rl_mr_unorm16(m.view_share)|(pack2x16float(vec2<f32>(hint,0.0))<<16u),r.b.w),
        r.c);
}
// What a later pass reads instead of decoding the hit: the receiver (words 1 and 2), MQ2's
// hint (word 1), the interpolated vertex normal (word 2).
fn rl_mr_receiver(b:vec4<u32>,c:vec4<u32>,pixel:vec2<i32>)->Receiver {
    return Receiver(bitcast<vec3<f32>>(c.xyz),rl_mr_oct_decode(b.x),pixel,b.w&0x0fffffffu,rl_mr_epsilon_decode(c.w&255u),(b.w&(1u<<29u))!=0u);
}
// The hit is near a triangle edge: the sun path's edge repair may move its point there.
fn rl_mr_near(b:vec4<u32>)->bool { return (b.w&(1u<<28u))!=0u; }
// No hit at this pixel this frame: the trace kernels' miss sentinel (slot all ones).
fn rl_mr_miss(b:vec4<u32>)->bool { return (b.w&0x0fffffffu)==0x0fffffffu; }
fn rl_mr_hint(b:vec4<u32>)->f32 { return unpack2x16float(b.z>>16u).x; }
fn rl_mr_vertex_normal(c:vec4<u32>)->vec3<f32> { return rl_mr_oct12_decode(c.w>>8u); }
fn rl_mr_unpack(a:vec4<u32>,b:vec4<u32>)->MaterialRecord {
    let scalars=unpack4x8unorm(a.z);
    let base=vec3<f32>(rl_mr_unorm16_decode(a.y),rl_mr_unorm16_decode(a.y>>16u),rl_mr_unorm16_decode(a.z));
    return MaterialRecord(rl_mr_oct_decode(a.x),base,scalars.z,scalars.w,rl_mr_unorm16_decode(b.y>>16u),rl_mr_rgb9e5_decode(a.w)/lighting.exposure,rl_mr_unorm16_decode(b.z),f32((b.y>>8u)&255u)/255.0,b.y&255u);
}
// Shared-exponent rgb9e5 (as the D3D/Vulkan format), for emission.
fn rl_mr_rgb9e5(c:vec3<f32>)->u32 {
    let v=clamp(c,vec3<f32>(0.0),vec3<f32>(65408.0));
    let m=max(v.x,max(v.y,v.z));
    if m<=0.0 { return 0u; }
    var e=max(-16,i32(floor(log2(m))))+1;
    var s=exp2(f32(e-9));
    if u32(round(m/s))==512u { e+=1; s*=2.0; }
    let q=vec3<u32>(round(v/s));
    return min(q.x,511u)|(min(q.y,511u)<<9u)|(min(q.z,511u)<<18u)|(u32(e+15)<<27u);
}
fn rl_mr_rgb9e5_decode(u:u32)->vec3<f32> {
    let s=exp2(f32(i32(u>>27u)-15-9));
    return vec3<f32>(f32(u&511u),f32((u>>9u)&511u),f32((u>>18u)&511u))*s;
}
"#;
/// The writing side: the images and the store.
const MATERIAL_WRITE: &str = r#"@group(0) @binding(75) var rl_material:texture_storage_2d_array<rgba32uint,write>;
fn rl_material_store(pixel:vec2<i32>,m:MaterialRecord,input:SurfaceInput,near:bool,hint:f32,source_front:bool,lit_front:bool) {
    let w=rl_mr_pack(m,input,near,hint,source_front,lit_front);
    textureStore(rl_material,pixel,2,w.a);
    textureStore(rl_material,pixel,0,w.b);
    textureStore(rl_material,pixel,1,w.c);
}
"#;
/// The receiver the sun path builds for a hit (shaders/primary_penumbra.wgsl
/// `primary_penumbra_receiver`, body for body: `material_trace_receiver_matches_the_sun_path`):
/// the trace point, then the near-edge repair's correction. The receiver check
/// (`RENDERLAB_MATERIAL_DEBUG=receiver`) holds the record against it. The record itself stores
/// shading's receiver and marks near-edge hits (`rl_mr_near_edge`): carrying the repair's
/// ray-query code made the writer 0.56-0.74 ms slower on the M5 for a handful of pixels, so a
/// reader that needs the sun path's point repairs the marked pixels itself, as the sun path does.
pub(crate) const MATERIAL_TRACE_RECEIVER: &str = r#"fn rl_mr_trace_receiver_of(trace:PrimaryTrace,id:vec2<u32>,size:vec2<u32>)->Receiver {
    let p=vec2<i32>(id);
    let slot=trace.record.x-1u;
    var receiver=primary_receiver(trace.point,trace.positions,slot,p);
    if primary_near_edge(trace,receiver.epsilon) {
        let record=trace.record;
        let v=primary_triangle_vertices(primary_placements[slot]&0x1fffffffu,record.y);
        let bary=bitcast<vec2<f32>>(record.zw);let w=vec3<f32>(1.0-bary.x-bary.y,bary);
        let point=world_position(receiver_point(v[0],v[1],v[2],w),slot).xyz;
        let ray=primary_ray(id,size);let delta=ray.far-ray.near;
        let repaired=primary_repair(PrimaryTrace(record,point,v),ray.near,delta/length(delta));
        if any(repaired.record!=record) || any(repaired.point!=point) {
            receiver=primary_receiver(repaired.point,repaired.positions,slot,p);
        }
    }
    return receiver;
}
fn rl_mr_trace_receiver(record:vec4<u32>,point:vec3<f32>,positions:array<vec3<f32>,3>,id:vec2<u32>,size:vec2<u32>)->Receiver {
    return rl_mr_trace_receiver_of(PrimaryTrace(record,point,positions),id,size);
}
"#;
/// `RENDERLAB_MATERIAL_DEBUG=receiver`'s pixel (`material_debug`): the record's receiver against
/// the sun path's for a fresh camera ray.
const MATERIAL_RECEIVER_CHECK: &str = r#"// RENDERLAB_MATERIAL_DEBUG=receiver (program.rs material_debug).
fn rl_material_receiver_error(pixel:vec2<i32>,input:SurfaceInput,source_front:bool,lit_front:bool)->vec4<f32> {
    let b=textureLoad(rl_material,pixel,0,0);let c=textureLoad(rl_material,pixel,1,0);
    let r=rl_mr_receiver(b,c,pixel);
    // The reference is the sun path's: a fresh camera ray, then its edge repair.
    let id=vec2<u32>(pixel);let size=textureDimensions(rl_material);
    let d=rl_mr_reference_receiver(id,size,input);
    // Where the repair moved the point the record must either hold the repaired point (a
    // trace that wrote it after repairing) or mark the hit near an edge (+2000 if neither);
    // there the point, normal and epsilon are the reader's to repair, so they are not compared.
    let moved=rl_mr_reference_edge(id,size);let near=rl_mr_near(b);
    let error=select(abs(r.world-d.world),vec3<f32>(0.0),moved);
    let same=r.slot==d.slot && r.thin==d.thin && ((b.w>>30u)&1u)==u32(source_front) && (b.w>>31u)==u32(lit_front);
    // atan2 of the cross and dot products keeps small angles that acos rounds to zero.
    let angle=select(atan2(length(cross(r.geometric,d.geometric)),dot(r.geometric,d.geometric)),0.0,moved);
    let n=normalize(input.normal);let v=rl_mr_vertex_normal(c);
    let vertex=atan2(length(cross(v,n)),dot(v,n));
    // z is negative where the repair moved the point; w is negative on hits, so they are
    // told from misses (whose alpha is 0 or 1).
    // +4000 where the model is not 255 exactly on the pixels of programs without the mapping.
    // x is negative (minus 1e-6) on the pixels of programs without the mapping.
    let model_right=!RL_MATERIAL_MODEL_CHECKED || ((b.y&255u)==255u)==RL_MATERIAL_UNMAPPED;
    let x=max(error.x,max(error.y,error.z))+select(1000.0,0.0,same)+select(0.0,2000.0,moved && !near && any(r.world!=d.world))+select(4000.0,0.0,model_right);
    return vec4<f32>(select(x,-1e-6-x,RL_MATERIAL_UNMAPPED),select(abs(r.epsilon-d.epsilon),0.0,moved),select(angle,-1e-6-angle,moved),-1e-6-vertex);
}
"#;
/// The trace module's edge repair for a shading module (shaders/primary_trace.wgsl through
/// `primary_repair`, with `primary_raw_closest` for the receiver check, and
/// primary_interactive.wgsl's `primary_near_edge`): its functions and `primary_scene`, without
/// the trace's outputs and entry points or the vertex and index arrays, which the shading
/// module declares under the same names.
fn material_trace_helpers() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let trace = include_str!("../shaders/primary_trace.wgsl");
        let scene = trace
            .lines()
            .find(|l| l.contains("var primary_scene:"))
            .expect("primary_trace.wgsl declares primary_scene");
        let start = trace.find("struct PrimaryTrace").expect("primary_trace.wgsl's PrimaryTrace");
        let end = trace.find("fn primary_scene_trace(").expect("primary_trace.wgsl's scene trace");
        let functions: Vec<&str> = trace[start..end]
            .lines()
            .filter(|l| {
                !l.starts_with("@group(")
                    && !l.starts_with("struct PrimaryVertices")
                    && !l.starts_with("struct PrimaryIndices")
            })
            .collect();
        let interactive = include_str!("../shaders/primary_interactive.wgsl");
        let near = interactive.find("fn primary_near_edge(").expect("primary_near_edge");
        let near_end = near + interactive[near..].find("\n}\n").expect("primary_near_edge's end") + 3;
        format!(
            "// RENDERLAB_MATERIAL_PASS: the trace module's edge repair (program.rs material_trace_helpers).\n{scene}\n{}\n{}",
            functions.join("\n"),
            &interactive[near..near_end]
        )
    })
}
/// The writer's near-edge mark: primary_interactive.wgsl's `primary_near_edge` on the hit
/// (ray-traced heads; arithmetic only), or never (heads without camera rays).
fn material_near_edge(rays: bool) -> String {
    if !rays {
        return "fn rl_mr_near_edge(record:vec4<u32>,point:vec3<f32>,positions:array<vec3<f32>,3>,epsilon:f32)->bool { return false; }\n".to_owned();
    }
    let interactive = include_str!("../shaders/primary_interactive.wgsl");
    let near = interactive.find("fn primary_near_edge(").expect("primary_near_edge");
    let near_end = near + interactive[near..].find("\n}\n").expect("primary_near_edge's end") + 3;
    format!(
        "struct RlMaterialTrace {{record:vec4<u32>,point:vec3<f32>,positions:array<vec3<f32>,3>}};\n{}\
         fn rl_mr_near_edge(record:vec4<u32>,point:vec3<f32>,positions:array<vec3<f32>,3>,epsilon:f32)->bool {{ return rl_mr_primary_near_edge(RlMaterialTrace(record,point,positions),epsilon); }}\n",
        interactive[near..near_end]
            .replacen("fn primary_near_edge(trace:PrimaryTrace,", "fn rl_mr_primary_near_edge(trace:RlMaterialTrace,", 1)
    )
}
/// The reading side: the images and the load.
const MATERIAL_READ: &str = r#"@group(0) @binding(75) var rl_material:texture_2d_array<u32>;
// The record's shading normal and MQ2 hint, for the engine code after `surface` (the
// specular filter's per-light `material_normal`, the entry's TR1 store).
var<private> rl_material_n:vec3<f32>;
var<private> rl_material_view_hint:f32;
var<private> rl_material_use:bool;
fn rl_material_load(pixel:vec2<i32>)->MaterialRecord {
    let b=textureLoad(rl_material,pixel,0,0);
    let m=rl_mr_unpack(textureLoad(rl_material,pixel,2,0),b);
    rl_material_n=m.normal;
    rl_material_view_hint=rl_mr_hint(b);
    return m;
}
"#;
/// The material pass's (`Write`) or the record-reading shading's (`Read`) variant of a traced
/// tiles or list module of `program` (`compose`, `compose_list`). The entry's `surface`
/// evaluation is replaced: `Write` stores the record and returns there; `Read` rebuilds the
/// surface from it. A program without the record mapping (`material_out` / `material_in`) gets
/// a receiver-only writer (every hit has a receiver for the passes after this one: model 255,
/// the interpolated normal as its shading normal, the material fields zero; its `surface` does
/// not run) and no reader (None: it keeps its single shading kernel). The edits follow the
/// program, so its range is unchanged.
pub(crate) fn material_variant(
    composed: Composed,
    program: &str,
    role: MaterialRole,
) -> Option<Composed> {
    material_variant_with(composed, program, role, material_debug(), material_probe())
}
/// `material_variant` with the debug view and the probe given rather than read from the
/// environment (tests compose every probe).
pub(crate) fn material_variant_with(
    mut composed: Composed,
    program: &str,
    role: MaterialRole,
    debug: Option<&str>,
    probe: Option<MaterialProbe>,
) -> Option<Composed> {
    let mapped = declares(program, "fn", "material_out") && declares(program, "fn", "material_in");
    // The receiver check also covers programs without the mapping: their debug reader writes
    // the check in place of their own shading.
    let receiver_check = debug.is_some_and(|d| d.contains("rl_material_receiver_error"));
    if !mapped && role == MaterialRole::Read && !receiver_check {
        return None;
    }
    let entry = composed.source.find("fn primary_shade_pixel(")?;
    let tail = &composed.source[entry..];
    if tail.matches(MATERIAL_SEAM).count() != 1 {
        return None;
    }
    let at = entry + tail.find(MATERIAL_SEAM).expect("counted");
    let replace = match (role, debug) {
        (MaterialRole::Write, _) if !mapped => MATERIAL_RECEIVER_ONLY.to_owned(),
        // The receiver check reads words 1 and 2 only (word 0 is unwritten under `tracetail`).
        (MaterialRole::Read, Some(field)) if !mapped || receiver_check => format!(
            "    textureStore(primary_hdr,pixel,{field});\n    return;\n    let surface_output=surface(surface_input);\n"
        ),
        (MaterialRole::Write, _) if probe == Some(MaterialProbe::NoStore) => "    let surface_output=surface(surface_input);\n    rl_material_probe_store(pixel,material_out(surface_output,surface_input),surface_input);\n    return;\n".to_owned(),
        (MaterialRole::Write, _) if probe == Some(MaterialProbe::Decode) => "    if rl_material_probe_input(surface_input)==0x9e3779b9u { textureStore(rl_material,pixel,2,vec4<u32>(1u)); textureStore(rl_material,pixel,0,vec4<u32>(1u)); textureStore(rl_material,pixel,1,vec4<u32>(1u)); }\n    return;\n    let surface_output=surface(surface_input);\n".to_owned(),
        (MaterialRole::Write, _) => "    let surface_output=surface(surface_input);\n    rl_material_store(pixel,material_out(surface_output,surface_input),surface_input,rl_mr_near_edge(record,receiver_world,array<vec3<f32>,3>(ta.position,tb.position,tc.position),surface_input.receiver.epsilon),tr1_view_hint(surface_output,surface_input,view_ray),source_front,lit_front);\n    return;\n".to_owned(),
        (MaterialRole::Read, None) if probe == Some(MaterialProbe::LoadPlus) => "    var surface_output=surface(surface_input);\n    let rl_probe=rl_material_load(pixel);\n    if rl_material_probe_record(rl_probe)==0x9e3779b9u { surface_output=material_in(rl_probe,surface_input); }\n".to_owned(),
        (MaterialRole::Read, None) if probe == Some(MaterialProbe::ReadPos) => "    if rl_material_probe_position(surface_input,receiver_world)==0x9e3779b9u { textureStore(primary_hdr,pixel,vec4<f32>(0.0)); }\n    return;\n    let surface_output=material_in(rl_material_load(pixel),surface_input);\n".to_owned(),
        (MaterialRole::Read, None) => "    let surface_output=material_in(rl_material_load(pixel),surface_input);\n".to_owned(),
        // The debug view: the record's field as the pixel's colour, before any lighting.
        (MaterialRole::Read, Some(field)) => format!(
            "    let rl_record=rl_material_load(pixel);\n    textureStore(primary_hdr,pixel,{field});\n    return;\n    let surface_output=material_in(rl_record,surface_input);\n"
        ),
    };
    composed
        .source
        .replace_range(at..at + MATERIAL_SEAM.len(), &replace);
    if role == MaterialRole::Read && mapped {
        composed = material_reader_edits(composed, entry)?;
    }
    // The reader's filter edit precedes the entry and moves it.
    let entry = composed.source.find("fn primary_shade_pixel(")?;
    if role == MaterialRole::Read && probe == Some(MaterialProbe::NoDecode) && mapped {
        composed = material_no_decode(composed, entry)?;
    }
    if role == MaterialRole::Read && probe == Some(MaterialProbe::ReadLean) {
        for (from, to) in MATERIAL_PROBE_LEAN {
            let at = entry + composed.source[entry..].find(from)?;
            composed.source.replace_range(at..at + from.len(), to);
        }
    }
    if role == MaterialRole::Write && probe == Some(MaterialProbe::Empty) {
        // Before the hit's decode: only the dispatch and the list walk remain.
        let start = entry + composed.source[entry..].find('{')? + 1;
        composed
            .source
            .insert_str(start, "\n    if size.x<0x7fffffffu { return; }");
    }
    composed.source.push_str(MATERIAL_PACKING);
    composed.source.push_str(match role {
        MaterialRole::Write => MATERIAL_WRITE,
        MaterialRole::Read => MATERIAL_READ,
    });
    let rays = composed.source.contains("enable wgpu_ray_query_vertex_return;")
        && composed.source.contains("var<storage,read> primary_indices:");
    if role == MaterialRole::Write {
        composed.source.push_str(&material_near_edge(rays));
    }
    // The receiver check traces a fresh camera ray and repairs it as the sun path does: the
    // trace module's helpers, only in that debug view's module.
    if role == MaterialRole::Read && receiver_check {
        composed.source.push_str(&format!(
            "const RL_MATERIAL_UNMAPPED:bool={};\nconst RL_MATERIAL_MODEL_CHECKED:bool={};\n",
            !mapped,
            probe != Some(MaterialProbe::TraceTail)
        ));
        // Misses report whether word 1 holds the miss sentinel: x 0 if so, 9000 if not; w 3.
        const MISS: &str = "textureStore(primary_hdr,pixel,primary_miss(id,size,primary_frame.clear));return;}";
        let at = entry + composed.source[entry..].find(MISS)?;
        composed.source.replace_range(
            at..at + MISS.len(),
            "textureStore(primary_hdr,pixel,vec4<f32>(select(9000.0,0.0,rl_mr_miss(textureLoad(rl_material,pixel,0,0))),0.0,0.0,3.0));return;}",
        );
        if rays {
            composed.source.push_str(material_trace_helpers());
            // The soft head's own rays need no `ray_query`; the repair's probes do. The
            // directive precedes everything, so the program's range moves by its length.
            if !composed.source.contains("enable wgpu_ray_query;") {
                const ENABLE: &str = "enable wgpu_ray_query;\n";
                composed.source.insert_str(0, ENABLE);
                composed.program = composed.program.start + ENABLE.len()..composed.program.end + ENABLE.len();
            }
            composed.source.push_str(MATERIAL_TRACE_RECEIVER);
            composed.source.push_str(MATERIAL_RECEIVER_CHECK);
            composed.source.push_str(
                "fn rl_mr_reference_receiver(id:vec2<u32>,size:vec2<u32>,input:SurfaceInput)->Receiver { return rl_mr_trace_receiver_of(primary_raw_closest(id,size),id,size); }\n\
                 fn rl_mr_reference_edge(id:vec2<u32>,size:vec2<u32>)->bool { let raw=primary_raw_closest(id,size); return raw.record.x!=0u && any(rl_mr_trace_receiver_of(raw,id,size).world!=raw.point); }\n",
            );
        } else {
            composed.source.push_str(
                "fn rl_mr_reference_receiver(id:vec2<u32>,size:vec2<u32>,input:SurfaceInput)->Receiver { return input.receiver; }\n\
                 fn rl_mr_reference_edge(id:vec2<u32>,size:vec2<u32>)->bool { return false; }\n",
            );
            composed.source.push_str(MATERIAL_RECEIVER_CHECK);
        }
    }
    if probe.is_some() {
        composed.source.push_str(MATERIAL_PROBE);
        if role == MaterialRole::Write {
            composed.source.push_str(MATERIAL_PROBE_STORE);
        }
    }
    Some(composed)
}
/// The writer's seam for a program without the record mapping: the receiver words, model 255,
/// the interpolated normal (the geometric one where it is degenerate) as the shading normal,
/// zero material fields and a neutral MQ2 hint; its shading evaluates `surface` itself.
const MATERIAL_RECEIVER_ONLY: &str = "    let rl_n=select(normalize(surface_input.normal),surface_input.geometric,dot(surface_input.normal,surface_input.normal)<=1e-12);\n    rl_material_store(pixel,MaterialRecord(rl_n,vec3<f32>(0.0),0.0,0.0,0.0,vec3<f32>(0.0),0.0,0.0,255u),surface_input,rl_mr_near_edge(record,receiver_world,array<vec3<f32>,3>(ta.position,tb.position,tc.position),surface_input.receiver.epsilon),1.0,source_front,lit_front);\n    return;\n    let surface_output=surface(surface_input);\n";
/// The specular filter's per-light evaluation of the receiver's mapped normal (normal map and
/// decal normals), which the reader takes from the record's shading normal instead.
const MATERIAL_FILTER_NORMAL: &str =
    "let n=material_normal(input);let v=unit_normal(input.eye-input.world);";
/// The entry's light setup, after the receiver's reflection hits are shaded (they go through
/// the same filter with their own surfaces): from here on the filter's surface is the receiver.
const MATERIAL_LIGHTS_SEAM: &str = "    rl_lights_prepare(surface_input.world);\n";
/// The entry's MQ2 hint, which the material pass computes and the record carries.
const MATERIAL_HINT_SEAM: &str = "tr1_view_hint(surface_output,surface_input,view_ray)";
/// The reader's edits outside the seam: the specular filter takes the receiver's mapped normal
/// from the record once the entry reaches `shade` (`rl_material_use`), and the entry stores
/// the record's MQ2 hint. The filter precedes the program, so its range moves with the edit.
/// None when a seam is missing (the caller then keeps the single kernel).
fn material_reader_edits(mut composed: Composed, entry: usize) -> Option<Composed> {
    let tail = &composed.source[entry..];
    if tail.matches(MATERIAL_LIGHTS_SEAM).count() != 1 || tail.matches(MATERIAL_HINT_SEAM).count() != 1 {
        return None;
    }
    let lights = entry + tail.find(MATERIAL_LIGHTS_SEAM).expect("counted");
    composed.source.insert_str(lights + MATERIAL_LIGHTS_SEAM.len(), "    rl_material_use=true;\n");
    let hint = entry + composed.source[entry..].find(MATERIAL_HINT_SEAM).expect("counted");
    composed.source.replace_range(hint..hint + MATERIAL_HINT_SEAM.len(), "rl_material_view_hint");
    if let Some(at) = composed.source[..composed.program.start].find(MATERIAL_FILTER_NORMAL) {
        let with = "var n:vec3<f32>;if rl_material_use {n=rl_material_n;} else {n=material_normal(input);}let v=unit_normal(input.eye-input.world);";
        composed.source.replace_range(at..at + MATERIAL_FILTER_NORMAL.len(), with);
        let shift = with.len() - MATERIAL_FILTER_NORMAL.len();
        composed.program = composed.program.start + shift..composed.program.end + shift;
    }
    Some(composed)
}
/// `RENDERLAB_MATERIAL_PROBE=tracetail` (`MaterialProbe`): the trace kernels write the receiver
/// words, and the material pass's own writer and reader kernels are not used.
pub(crate) fn material_trace_tail() -> bool {
    material_probe() == Some(MaterialProbe::TraceTail)
}
/// `RENDERLAB_MATERIAL_DEBUG=receiver`: the reader runs the receiver check (also under
/// `tracetail`, where no other reader runs).
pub(crate) fn material_receiver_check() -> bool {
    material_debug().is_some_and(|d| d.contains("rl_material_receiver_error"))
}
/// A WGSL function's text, from `fn name(` to its closing brace, in `text`.
fn wgsl_function<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("fn {name}"));
    let end = start + text[start..].find("\n}\n").unwrap_or_else(|| panic!("fn {name}'s end")) + 3;
    &text[start..end]
}
/// The camera-hit trace kernels' record writes (`backend_ara::primary_trace`, with the pass on):
/// the miss sentinel on every pixel without a hit, the region's exterior included (the trace
/// dispatch covers the whole image), and with `tracetail` the receiver words on hits. Uses the
/// record's own encoders (`MATERIAL_PACKING`).
pub(crate) fn material_trace_writes() -> String {
    let line = |prefix: &str| -> &str {
        MATERIAL_PACKING
            .lines()
            .find(|l| l.starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix}"))
    };
    format!(
        r#"// RENDERLAB_MATERIAL_PASS (program.rs material_trace_writes): the record's receiver words.
@group(0) @binding(75) var rl_material:texture_storage_2d_array<rgba32uint,write>;
{}{}{}{}
{}
{}
// The miss sentinel (`rl_mr_miss`).
fn rl_receiver_miss(p:vec2<i32>) {{ textureStore(rl_material,p,0,RL_MR_MISS_WORD1); textureStore(rl_material,p,1,vec4<u32>(0u)); }}
// The interpolated vertex normal of a trace's hit, as shading's entry builds it, from the
// vertex, index and normal-transform arrays (producers that have them).
fn rl_mr_trace_vertex_normal(trace:PrimaryTrace)->vec3<f32> {{
    let slot=trace.record.x-1u;let batch=primary_placements[slot]&0x1fffffffu;let base=trace.record.y*3u;
    let bary=bitcast<vec2<f32>>(trace.record.zw);let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let m=normals[slot];let nm=mat3x3<f32>(m.x.xyz,m.y.xyz,m.z.xyz);
    var n=vec3<f32>(0.0);
    for(var k=0u;k<3u;k+=1u) {{
        let i=primary_indices[batch].data[base+k];
        let a=primary_vertices[batch].data[i*3u];let b=primary_vertices[batch].data[i*3u+1u];
        n+=unit_normal(nm*unit_normal(vec3<f32>(a.w,b.xy)))*w[k];
    }}
    return n;
}}
// A hit's receiver words from the trace: its point is the sun path's (the repair kernel
// rewrites the hits it moves), so no hit is marked near an edge. Both sides of the shading
// entry are one value (its lit front reduces to its source front), tested against the near
// plane's point on this pixel's camera ray, which is on the eye's side of the receiver plane.
fn rl_receiver_store(p:vec2<i32>,trace:PrimaryTrace,receiver:Receiver) {{
    let front=dot(receiver.geometric,primary_ray(vec2<u32>(p),vec2<u32>(textureDimensions(rl_material))).near-receiver.world)>=0.0;
    let w=rl_mr_receiver_words(receiver,rl_mr_trace_vertex_normal(trace),front,front,false);
    textureStore(rl_material,p,0,w.b);
    textureStore(rl_material,p,1,w.c);
}}
"#,
        wgsl_function(MATERIAL_PACKING, "rl_mr_oct"),
        wgsl_function(MATERIAL_PACKING, "rl_mr_oct12"),
        wgsl_function(MATERIAL_PACKING, "rl_mr_epsilon"),
        line("struct RlReceiverWords"),
        wgsl_function(MATERIAL_PACKING, "rl_mr_receiver_words"),
        line("const RL_MR_MISS_WORD1"),
    )
}
/// The local-light visibility pass's per-pixel helper (`light_vis_pass`, `rl_lvis_pixel`) built
/// from the material record's receiver layers instead of decoding the hit: the receiver, the
/// world point and the pixel are all the pass reads (`light_sample_untinted`,
/// `light_visibility`). Pixels whose word 1 holds the miss sentinel are skipped. Word 1's slot
/// is the receiver's batch (`primary_receiver`: `primary_placements[placement]&0x1fffffff`), so
/// it indexes the materials directly. None when the module has no `rl_lvis_pixel` (the pass is
/// off).
pub(crate) fn light_vis_from_record(mut source: String) -> Option<String> {
    let start = source.find("fn rl_lvis_pixel(")?;
    const END: &str = "    rl_lvis_store(surface_input);\n}\n";
    let end = start + source[start..].find(END)? + END.len();
    source.replace_range(
        start..end,
        "fn rl_lvis_pixel(id:vec2<u32>,size:vec2<u32>) {\n\
         \x20   let pixel=vec2<i32>(id);\n\
         \x20   let b=textureLoad(rl_material,pixel,0,0);\n\
         \x20   if rl_mr_miss(b) { return; }\n\
         \x20   let material=primary_materials[b.w&0x0fffffffu].value;\n\
         \x20   rl_lvis_store(rl_material_input(pixel,id,size,material));\n\
         }\n",
    );
    source.push_str(MATERIAL_PACKING);
    source.push_str(MATERIAL_READ);
    source.push_str(MATERIAL_NO_DECODE);
    Some(source)
}
/// The `nodecode` probe's entry edits: the `SurfaceInput` from the record, and the entry's
/// later reads of the decoded point and side from it, so the vertex fetch is dead code.
fn material_no_decode(mut composed: Composed, entry: usize) -> Option<Composed> {
    let start = entry + composed.source[entry..].find("    let surface_input=decal_prepare(SurfaceInput(")?;
    let end = start + composed.source[start..].find("decal_none()));\n")? + "decal_none()));\n".len();
    composed.source.replace_range(start..end, "    let surface_input=rl_material_input(pixel,id,size,material);\n");
    let tail_start = start;
    let tail_end = tail_start + composed.source[tail_start..].find("\n}\n")?;
    let tail = composed.source[tail_start..tail_end]
        .replace("surface_input.world", "\u{1}")
        .replace("input.world", "surface_input.world")
        .replace('\u{1}', "surface_input.world")
        .replace("!lit_front", "!surface_input.source_front");
    composed.source.replace_range(tail_start..tail_end, &tail);
    composed.source.push_str(MATERIAL_NO_DECODE);
    Some(composed)
}
const MATERIAL_NO_DECODE: &str = r#"// RENDERLAB_MATERIAL_PROBE=nodecode: the reader's SurfaceInput from the record alone.
fn rl_material_plane_point(p:vec2<u32>,size:vec2<u32>,point:vec3<f32>,n:vec3<f32>)->vec3<f32> {
    let ray=primary_ray(p,size);let d=ray.far-ray.near;
    return ray.near+d*(dot(point-ray.near,n)/dot(d,n));
}
fn rl_material_input(pixel:vec2<i32>,id:vec2<u32>,size:vec2<u32>,material:Material)->SurfaceInput {
    let b=textureLoad(rl_material,pixel,0,0);let c=textureLoad(rl_material,pixel,1,0);
    let r=rl_mr_receiver(b,c,pixel);
    let source_front=((b.w>>30u)&1u)!=0u;let lit_front=(b.w>>31u)!=0u;
    let rg=select(r.geometric,-r.geometric,source_front);
    let geometric=select(rg,-rg,lit_front);
    let dx=rl_material_plane_point(id+vec2<u32>(1u,0u),size,r.world,r.geometric)-r.world;
    let dy=rl_material_plane_point(id+vec2<u32>(0u,1u),size,r.world,r.geometric)-r.world;
    let n=rl_mr_vertex_normal(c);
    let t=normalize(select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(n.x)>0.9)-n*dot(n,select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(n.x)>0.9)));
    return SurfaceInput(r.world,dx,dy,geometric,n,vec3<f32>(0.0),vec3<f32>(0.0),t,1.0,vec2<f32>(0.0),vec2<f32>(0.0),vec2<f32>(0.0),vec4<f32>(1.0),
        lit_front,lighting.eye,id,material,r,decal_none());
}
"#;
/// `RENDERLAB_MATERIAL_PROBE` (measurement only, with the pass on): variants that split the
/// material pass's cost. The writer's `empty` returns before the hit's decode (dispatch and list
/// walk); `decode` decodes the hit and stops before `surface`; `nostore` evaluates and packs the
/// record but does not store it. The reader's `loadplus` evaluates `surface` as the single
/// kernel does and also loads and unpacks the record; `readlean` shades as usual but with
/// the hit's vertex attributes (uv, colour, normal, tangent, their derivatives, decals)
/// constant, so only the position side of the decode is left; `readpos` returns after
/// that position side (world point, footprint, geometric normal, shadow receiver); `nodecode`
/// builds the reader's `SurfaceInput` from words 1 and 2 alone, with no vertex fetch (the
/// footprint from the neighbouring pixels' rays against the receiver's plane; the normal's
/// derivatives, tangent and uv zero), the most a decode-free reader can save; its picture is
/// wrong where curvature, tangents or uv matter. `tracetail` drops the material pass's
/// kernels: the camera-hit trace writes words 1 and 2 itself (`material_trace_tail`) and
/// shading keeps its single kernel. Each keeps its work live by comparing a
/// hash of its result with a value it never takes; the pictures are wrong by design.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MaterialProbe {
    Empty,
    Decode,
    NoStore,
    LoadPlus,
    ReadLean,
    ReadPos,
    NoDecode,
    TraceTail,
}
fn material_probe() -> Option<MaterialProbe> {
    static PROBE: std::sync::OnceLock<Option<MaterialProbe>> = std::sync::OnceLock::new();
    *PROBE.get_or_init(
        || match std::env::var("RENDERLAB_MATERIAL_PROBE").ok()?.as_str() {
            "empty" => Some(MaterialProbe::Empty),
            "decode" => Some(MaterialProbe::Decode),
            "nostore" => Some(MaterialProbe::NoStore),
            "loadplus" => Some(MaterialProbe::LoadPlus),
        "readlean" => Some(MaterialProbe::ReadLean),
        "readpos" => Some(MaterialProbe::ReadPos),
        "nodecode" => Some(MaterialProbe::NoDecode),
        "tracetail" => Some(MaterialProbe::TraceTail),
            _ => None,
        },
    )
}
/// The probes' hashes. Only the role's own functions are called, so the others are not
/// referenced and the reader's module does not see the writer's images.
const MATERIAL_PROBE: &str = r#"// RENDERLAB_MATERIAL_PROBE (program.rs material_probe): measurement only.
fn rl_probe3(v:vec3<f32>)->u32 { let u=bitcast<vec3<u32>>(v); return u.x^(u.y*3u)^(u.z*5u); }
fn rl_probe2(v:vec2<f32>)->u32 { let u=bitcast<vec2<u32>>(v); return u.x^(u.y*7u); }
fn rl_material_probe_input(s:SurfaceInput)->u32 {
    return rl_probe3(s.world)^rl_probe3(s.world_dx)^rl_probe3(s.world_dy)^rl_probe3(s.geometric)
        ^rl_probe3(s.normal)^rl_probe3(s.normal_dx)^rl_probe3(s.normal_dy)^rl_probe3(s.tangent)
        ^bitcast<u32>(s.handedness)^rl_probe2(s.uv)^rl_probe2(s.uv_dx)^rl_probe2(s.uv_dy)
        ^rl_probe3(s.color.xyz)^bitcast<u32>(s.color.w)^rl_probe3(s.material.base_color.xyz);
}
fn rl_material_probe_position(s:SurfaceInput,receiver:vec3<f32>)->u32 {
    return rl_probe3(s.world)^rl_probe3(s.world_dx)^rl_probe3(s.world_dy)^rl_probe3(s.geometric)^rl_probe3(receiver);
}
fn rl_material_probe_record(m:MaterialRecord)->u32 {
    return rl_probe3(m.normal)^rl_probe3(m.base)^rl_probe3(m.emission)^bitcast<u32>(m.alpha)
        ^bitcast<u32>(m.metallic)^bitcast<u32>(m.roughness)^bitcast<u32>(m.view_share)
        ^bitcast<u32>(m.view_alpha)^m.model;
}
"#;
/// `readlean`: the traced entry's SurfaceInput without the vertex attributes and decals.
const MATERIAL_PROBE_LEAN: [(&str, &str); 3] = [
    ("let surface_input=decal_prepare(SurfaceInput(", "let surface_input=(SurfaceInput("),
    (
        "source_geometric,input.normal,normal_dx,normal_dy,input.tangent,input.handedness,",
        "source_geometric,source_geometric,vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(1.0,0.0,0.0),1.0,",
    ),
    (
        "input.uv,gradients[0],gradients[1],input.color,",
        "vec2<f32>(0.5),vec2<f32>(0.0),vec2<f32>(0.0),vec4<f32>(1.0),",
    ),
];
/// `nostore`: `rl_material_store`'s packing, stored only on a hash it never takes.
const MATERIAL_PROBE_STORE: &str = r#"fn rl_material_probe_store(pixel:vec2<i32>,m:MaterialRecord,input:SurfaceInput) {
    let w=rl_mr_pack(m,input,false,1.0,true,true);
    let h=w.a^(w.b*vec4<u32>(3u,5u,7u,11u))^(w.c*vec4<u32>(13u,17u,19u,23u));
    if (h.x^h.y^h.z^h.w)==0x9e3779b9u { textureStore(rl_material,pixel,2,w.a); textureStore(rl_material,pixel,0,w.b); textureStore(rl_material,pixel,1,w.c); }
}
"#;
// ---- end of material pass ----
