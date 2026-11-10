//! WGSL resource-interface adapter to Ara's runtime descriptor arrays and push roots.
use super::{Pass, SpirvStage, parse, validate};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;

/// Compile WGSL math with heap-indexed bindings. The tone LUT's temporary binding
/// is aliased onto sampled binding 5 in the emitted decorations, because naga's
/// validator rejects different typed declarations sharing one source binding.
pub fn spirv(pass: Pass) -> Result<Vec<SpirvStage>> {
    from_source(pass, pass.source().as_ref())
}
pub(crate) fn from_source(pass: Pass, wgsl: &str) -> Result<Vec<SpirvStage>> {
    let source = hoist_directives(&adapt(pass, wgsl)?);
    let mut module = parse(&source)?;
    super::select(&mut module, pass)?;
    let entries: Vec<_> = if pass.raster() {
        vec![
            (naga::ShaderStage::Vertex, pass.vertex()),
            (naga::ShaderStage::Fragment, pass.entry()),
        ]
    } else {
        vec![(naga::ShaderStage::Compute, pass.entry())]
    };
    entries
        .into_iter()
        .map(|(stage, entry)| {
            // The cooked Vulkan ABI requires native main; retain the source name
            // separately for diagnostics and artifact names, as on the PC replay.
            let mut selected = module.clone();
            selected
                .entry_points
                .iter_mut()
                .find(|e| e.stage == stage && e.name == entry)
                .context("selected SPIR-V entry missing")?
                .name = "main".into();
            let info = validate(&selected)?;
            let mut words = naga::back::spv::write_vec(
                &selected,
                &info,
                &naga::back::spv::Options {
                    lang_version: (1, 5),
                    // Keep Naga's existing unchecked SPIR-V policy: enabling
                    // checks hits its descriptor-wrapper inference and unsigned
                    // vector constant bugs. Metal checks are enabled separately.
                    flags: naga::back::spv::Options::default().flags
                        | naga::back::spv::WriterFlags::DEBUG,
                    ..Default::default()
                },
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: stage,
                    entry_point: "main".into(),
                }),
            )?;
            // Naga 30 emits OpTypeRuntimeArray for descriptor binding arrays but does
            // not declare RuntimeDescriptorArray when their indices are uniform. The
            // array declaration itself requires it (Vulkan VUID 04680), independently
            // of nonuniform indexing. SPIR-V 1.5 makes the capability core.
            let mut capability_end = 5;
            let mut runtime_descriptors = false;
            while capability_end < words.len() && words[capability_end] & 0xffff == 17 {
                ensure!(words[capability_end] >> 16 == 2, "invalid OpCapability");
                runtime_descriptors |= words[capability_end + 1] == 5302;
                capability_end += 2;
            }
            if !runtime_descriptors {
                words.splice(capability_end..capability_end, [(2 << 16) | 17, 5302]);
            }
            super::spv_nonuniform::repair(&mut words)?;
            remap_heap_aliases(&mut words)?;
            validate_heap_bindings(&words)?;
            repair_image_arguments(&mut words)?;
            super::spv_layout::repair(&mut words)?;
            Ok(SpirvStage {
                stage,
                entry: entry.into(),
                words,
            })
        })
        .collect()
}
fn adapt(pass: Pass, wgsl: &str) -> Result<String> {
    // Select before interface adaptation: unreferenced program/shadow bindings
    // must not leak into a leaf's heap contract. Re-emission is structurally validated;
    // native Vulkan replay of the complete cohort is still required.
    let mut module = parse(wgsl)?;
    super::select(&mut module, pass)?;
    let info = validate(&module)?;
    let source =
        naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())?;
    let original = parse(&source)?;
    let body = strip_bindings(&source);
    if pass.indirect() {
        super::heap_interface::assert_source_contract(
            pass,
            wgsl,
            &super::heap_interface::from_module(&original)?,
        )?;
        return super::packet_spirv::adapt(&source, &original);
    }
    if matches!(pass, Pass::MeshDiagnostic) {
        return mesh_interface(&body, &original);
    }
    if matches!(pass, Pass::ParticleSort) {
        return particle_interface(&body, &original);
    }
    if pass.depth_input() {
        return surface_interface(&body, pass, &original);
    }
    if pass.skin() {
        return skin_interface(&body, &original);
    }
    if matches!(pass, Pass::LightCluster) {
        return cluster_interface(&body, &original);
    }
    if pass.atmosphere() {
        return atmosphere_interface(&body, pass, &original);
    }
    let names = BTreeMap::from([
        ("input_colour", "ara_textures[ara_root.w0]"),
        ("colour", "ara_textures[ara_root.w0]"),
        ("second_colour", "ara_textures[ara_root.w1]"),
        ("bloom", "ara_textures[ara_root.w1]"),
        ("auxiliary", "ara_textures[ara_root.w2]"),
        ("velocity_tiles", "ara_textures[ara_root.w3]"),
        ("output_colour", "ara_outputs[ara_root.w4]"),
        ("settings", "ara_settings[ara_root.w5]"),
        ("linear_sampler", "ara_samplers[ara_root.w6]"),
        ("lut", "ara_luts[ara_root.w7]"),
    ]);
    let out = rewrite_globals(body.as_ref(), &names, &original)?;
    let settings = if original
        .global_variables
        .iter()
        .any(|(_, g)| g.name.as_deref() == Some("settings"))
    {
        "@group(0) @binding(1) var<storage,read> ara_settings:binding_array<Settings>;"
    } else {
        ""
    };
    Ok(format!(
        r#"
enable wgpu_binding_array;
struct AraRoot {{w0:u32,w1:u32,w2:u32,w3:u32,w4:u32,w5:u32,w6:u32,w7:u32,}};
var<immediate> ara_root:AraRoot;
@group(0) @binding(5) var ara_textures:binding_array<texture_2d<f32>>;
@group(0) @binding(3) var ara_outputs:binding_array<texture_storage_2d<rgba16float,write>>;
@group(0) @binding(2) var ara_samplers:binding_array<sampler>;
{settings}
@group(0) @binding(7) var ara_luts:binding_array<texture_3d<f32>>;
{out}
"#
    ))
}

pub(super) fn strip_bindings(source: &str) -> String {
    let mut body = String::new();
    let mut binding = false;
    for line in source.lines() {
        if line.trim_start().starts_with("@group(") {
            binding = true;
        }
        if binding {
            if line.contains(';') {
                binding = false;
            }
            continue;
        }
        body.push_str(line);
        body.push('\n');
    }
    body
}

/// Naga 30 passes a loaded image value for binding-array elements although its
/// resource-parameter ABI expects the UniformConstant pointer. Repair only this
/// exact, type-checked mismatch; unexpected parameter mismatches remain errors.
fn repair_image_arguments(words: &mut [u32]) -> Result<()> {
    let mut result_types = BTreeMap::new();
    let mut pointers = BTreeMap::new();
    let mut loads = BTreeMap::new();
    let mut signatures = BTreeMap::new();
    let mut functions = BTreeMap::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        ensure!(
            count > 0 && at + count <= words.len(),
            "invalid SPIR-V instruction"
        );
        match words[at] & 0xffff {
            32 => {
                pointers.insert(words[at + 1], (words[at + 2], words[at + 3]));
            }
            33 => {
                signatures.insert(words[at + 1], words[at + 3..at + count].to_vec());
            }
            54 => {
                functions.insert(words[at + 2], words[at + 4]);
            }
            55 | 59 | 65 | 66 => {
                result_types.insert(words[at + 2], words[at + 1]);
            }
            61 => {
                result_types.insert(words[at + 2], words[at + 1]);
                loads.insert(words[at + 2], (words[at + 1], words[at + 3]));
            }
            _ => {}
        }
        at += count;
    }
    at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        if words[at] & 0xffff == 57 {
            let signature = functions
                .get(&words[at + 3])
                .and_then(|id| signatures.get(id))
                .context("function signature missing")?;
            ensure!(
                signature.len() == count - 4,
                "function argument count mismatch"
            );
            for (argument, expected) in words[at + 4..at + count].iter_mut().zip(signature) {
                if let Some(&(loaded_type, pointer)) = loads.get(argument)
                    && let Some(&(storage, pointee)) = pointers.get(expected)
                    && storage == 0
                    && pointee == loaded_type
                {
                    ensure!(
                        result_types.get(&pointer) == Some(expected),
                        "image pointer type mismatch"
                    );
                    *argument = pointer;
                }
            }
        }
        at += count;
    }
    Ok(())
}

fn cluster_interface(body: &str, original: &naga::Module) -> Result<String> {
    let names = BTreeMap::from([
        ("lights", "ara_lights[ara_root.w0].values"),
        ("clusters", "ara_clusters[ara_root.w1]"),
        ("frame", "ara_frames[ara_root.w2]"),
    ]);
    let out = rewrite_globals(body, &names, original)?;
    Ok(format!(
        r#"enable wgpu_binding_array;
struct AraRoot {{w0:u32,w1:u32,w2:u32,w3:u32,w4:u32,w5:u32,w6:u32,w7:u32,}};
var<immediate> ara_root:AraRoot;
struct AraLights {{values:array<Light>,}};
@group(0) @binding(1) var<storage,read> ara_lights:binding_array<AraLights>;
@group(0) @binding(8) var<storage,read> ara_frames:binding_array<LcFrame>;
@group(0) @binding(3) var ara_clusters:binding_array<texture_storage_2d<r32uint,write>>;
{out}
"#
    ))
}

fn atmosphere_interface(body: &str, pass: Pass, original: &naga::Module) -> Result<String> {
    let names = BTreeMap::from([
        ("atmosphere", "ara_atmospheres[ara_root.w0]"),
        ("atmosphere_transmittance", "ara_textures[ara_root.w1]"),
        ("atmosphere_multiscatter", "ara_textures[ara_root.w2]"),
        ("at_skyview", "ara_textures[ara_root.w3]"),
        ("atmosphere_skyview", "ara_textures[ara_root.w3]"),
        ("atmosphere_volume", "ara_luts[ara_root.w7]"),
        ("dfg_sampler", "ara_samplers[ara_root.w5]"),
        ("at_transmittance_out", "ara_outputs[ara_root.w4]"),
        ("at_multiscatter_out", "ara_outputs[ara_root.w4]"),
        ("at_skyview_out", "ara_outputs[ara_root.w4]"),
        ("atmosphere_sampler", "ara_samplers[ara_root.w5]"),
        ("at_specular_out", "ara_outputs[ara_root.w4]"),
        ("at_diffuse_out", "ara_outputs[ara_root.w4]"),
        ("at_volume_out", "ara_volume_outputs[ara_root.w4]"),
        ("at_scatter_out", "ara_volume_outputs[ara_root.w4]"),
        ("at_scatter", "ara_luts[ara_root.w7]"),
        ("at_history", "ara_luts[ara_root.w6]"),
    ]);
    let out = rewrite_globals(body, &names, original)?;
    let output = if pass.environment() {
        "texture_storage_2d_array<rgba32float,write>"
    } else {
        "texture_storage_2d<rgba16float,write>"
    };
    Ok(format!(
        r#"enable wgpu_binding_array;
struct AraRoot {{w0:u32,w1:u32,w2:u32,w3:u32,w4:u32,w5:u32,w6:u32,w7:u32,}};
var<immediate> ara_root:AraRoot;
@group(0) @binding(9) var ara_volume_outputs:binding_array<texture_storage_3d<rgba16float,write>>;
@group(0) @binding(7) var ara_luts:binding_array<texture_3d<f32>>;
@group(0) @binding(1) var<storage,read> ara_atmospheres:binding_array<AtmosphereUniform>;
@group(0) @binding(5) var ara_textures:binding_array<texture_2d<f32>>;
@group(0) @binding(3) var ara_outputs:binding_array<{output}>;
@group(0) @binding(2) var ara_samplers:binding_array<sampler>;
{out}
"#
    ))
}

// Scope descriptor aliases to their generated variable identities, rather than
// rewriting arbitrary variables merely because a binding number happens to match.
fn remap_heap_aliases(words: &mut [u32]) -> Result<()> {
    let mut aliases = BTreeMap::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        ensure!(
            count > 0 && at + count <= words.len(),
            "invalid SPIR-V instruction"
        );
        if words[at] & 0xffff == 5 && count >= 3 {
            let bytes: Vec<_> = words[at + 2..at + count]
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .take_while(|b| *b != 0)
                .collect();
            let name = std::str::from_utf8(&bytes)?;
            let remap = if let Some(n) = name
                .strip_prefix("ara_storage_")
                .and_then(|n| n.parse::<u32>().ok())
            {
                Some((1600 + n, 3))
            } else if let Some(n) = name
                .strip_prefix("ara_array_")
                .and_then(|n| n.parse::<u32>().ok())
            {
                Some((1900 + n, 1))
            } else if name == "ara_depths" {
                Some((11, 5))
            } else if name == "ara_volume_outputs" {
                Some((9, 3))
            } else if name == "ara_luts" {
                Some((7, 5))
            } else if name == "ara_frames" {
                Some((8, 1))
            } else if let Some(n) = name
                .strip_prefix("ara_buffer_")
                .and_then(|n| n.parse::<u32>().ok())
            {
                Some((400 + n, 1))
            } else if let Some(n) = name
                .strip_prefix("ara_image_")
                .and_then(|n| n.parse::<u32>().ok())
            {
                Some((700 + n, 5))
            } else if let Some(n) = name
                .strip_prefix("ara_sampler_")
                .and_then(|n| n.parse::<u32>().ok())
            {
                Some((1000 + n, 2))
            } else if let Some(n) = name
                .strip_prefix("ara_comparison_")
                .and_then(|n| n.parse::<u32>().ok())
            {
                Some((1300 + n, 6))
            } else if let Some(name) = name.strip_prefix("ara_mesh_") {
                super::MeshResource::from_name(name).map(|r| (300 + r as u32, 1))
            } else if let Some(name) = name.strip_prefix("ara_particle_") {
                super::ParticleResource::from_name(name).map(|r| (200 + r as u32, 1))
            } else if let Some(name) = name.strip_prefix("ara_skin_") {
                super::SkinResource::from_name(name).map(|r| (100 + r as u32, 1))
            } else {
                None
            };
            if let Some(remap) = remap {
                aliases.insert(words[at + 1], remap);
            }
        }
        at += count;
    }
    at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        if words[at] & 0xffff == 71
            && count == 4
            && words[at + 2] == 33
            && let Some(&(before, after)) = aliases.get(&words[at + 1])
        {
            ensure!(words[at + 3] == before, "unexpected heap alias binding");
            words[at + 3] = after;
        }
        at += count;
    }
    Ok(())
}
fn skin_interface(body: &str, original: &naga::Module) -> Result<String> {
    use super::SkinResource;
    let mut names = BTreeMap::new();
    let mut declarations = String::new();
    let mut wrappers = std::collections::BTreeSet::from(["AraWords"]);
    for resource in SkinResource::ALL {
        let name = resource.name();
        if !original
            .global_variables
            .iter()
            .any(|(_, g)| g.name.as_deref() == Some(name))
        {
            continue;
        }
        if !matches!(resource, SkinResource::Params) {
            wrappers.insert(resource.ty());
        }
        names.insert(
            name,
            format!(
                "ara_skin_{name}[{}]{}",
                resource.spirv_slot(),
                if matches!(resource, SkinResource::Params) {
                    ""
                } else {
                    ".values"
                }
            ),
        );
        declarations.push_str(&format!(
            "@group(0) @binding({}) var<storage,{}> ara_skin_{name}:binding_array<{}>;\n",
            100 + resource as u32,
            if resource.writes() {
                "read_write"
            } else {
                "read"
            },
            resource.ty()
        ));
    }
    let out = rewrite_globals(body, &names, original)?;
    let mut types = String::new();
    for (ty, element) in [
        ("AraFloats", "f32"),
        ("AraWords", "u32"),
        ("AraInfluences", "Influence"),
        ("AraMatrices", "mat4x4<f32>"),
        ("AraTangents", "vec4<f32>"),
        ("AraMappings", "vec2<u32>"),
    ] {
        if wrappers.contains(ty) {
            types.push_str(&format!("struct {ty} {{values:array<{element}>,}};\n"));
        }
    }
    Ok(format!(
        r#"enable wgpu_binding_array;
struct AraRoot {{w0:u32,w1:u32,w2:u32,w3:u32,w4:u32,w5:u32,w6:u32,w7:u32,}};
var<immediate> ara_root:AraRoot;
{types}
@group(0) @binding(1) var<storage,read> ara_skin_table:binding_array<AraWords>;
{declarations}
{out}
"#
    ))
}

/// Rewrite only globals registered by naga. Field selections/declarations keep
/// their names; a local shadow is rejected explicitly rather than silently changed.
pub(super) fn rewrite_globals<S: AsRef<str>>(
    body: &str,
    names: &BTreeMap<&str, S>,
    original: &naga::Module,
) -> Result<String> {
    let globals: std::collections::BTreeSet<_> = original
        .global_variables
        .iter()
        .filter_map(|(_, g)| g.name.as_deref())
        .filter(|n| names.contains_key(n))
        .collect();
    for function in original
        .functions
        .iter()
        .map(|(_, f)| f)
        .chain(original.entry_points.iter().map(|e| &e.function))
    {
        let shadow = function
            .arguments
            .iter()
            .filter_map(|a| a.name.as_deref())
            .chain(
                function
                    .local_variables
                    .iter()
                    .filter_map(|(_, v)| v.name.as_deref()),
            )
            .chain(function.named_expressions.values().map(String::as_str))
            .find(|n| globals.contains(n));
        ensure!(
            shadow.is_none(),
            "resource global is shadowed locally: {:?}",
            shadow
        );
    }
    let mut out = String::new();
    let mut at = 0;
    while at < body.len() {
        let c = body[at..]
            .chars()
            .next()
            .expect("at stays on character boundaries");
        if c.is_ascii_alphanumeric() || c == '_' {
            let start = at;
            while at < body.len()
                && (body.as_bytes()[at].is_ascii_alphanumeric() || body.as_bytes()[at] == b'_')
            {
                at += 1;
            }
            let word = &body[start..at];
            let member = body[..start].trim_end().ends_with('.');
            let declaration = body[at..].trim_start().starts_with(':');
            if !member && !declaration && globals.contains(word) {
                out.push_str(
                    names
                        .get(word)
                        .expect("registered resource global")
                        .as_ref(),
                );
            } else {
                out.push_str(word);
            }
        } else {
            out.push(c);
            at += c.len_utf8();
        }
    }
    Ok(out)
}

fn surface_interface(body: &str, pass: Pass, original: &naga::Module) -> Result<String> {
    let names = BTreeMap::from([
        ("scene_depth", "ara_depths[ara_root.w0]"),
        ("frame", "ara_post_frames[ara_root.w1]"),
        ("surface", "ara_surfaces[ara_root.w2]"),
        ("tp_raster_depth", "ara_depths[ara_root.w0]"),
        ("tp", "ara_post_frames[ara_root.w1]"),
        ("tp_depth_out", "ara_surfaces[ara_root.w2]"),
    ]);
    let out = rewrite_globals(body, &names, original)?;
    let depth = if matches!(pass, Pass::RasterSurfaceFour) {
        "texture_depth_multisampled_2d"
    } else {
        "texture_depth_2d"
    };
    let (frame, format) = if matches!(pass, Pass::TransparencyDepth) {
        ("TpFrame", "r32float")
    } else {
        ("PostFrame", "rgba32float")
    };
    Ok(format!(
        r#"enable wgpu_binding_array;
struct AraRoot {{w0:u32,w1:u32,w2:u32,w3:u32,w4:u32,w5:u32,w6:u32,w7:u32,}};
var<immediate> ara_root:AraRoot;
@group(0) @binding(11) var ara_depths:binding_array<{depth}>;
@group(0) @binding(1) var<storage,read> ara_post_frames:binding_array<{frame}>;
@group(0) @binding(3) var ara_surfaces:binding_array<texture_storage_2d<{format},write>>;
{out}
"#
    ))
}

fn particle_interface(body: &str, original: &naga::Module) -> Result<String> {
    let mut names = BTreeMap::new();
    let mut declarations = String::new();
    for r in super::ParticleResource::ALL {
        let name = r.name();
        names.insert(
            name,
            format!(
                "ara_particle_{name}[ara_root.w{}]{}",
                r as u32,
                if r.array() { ".values" } else { "" }
            ),
        );
        declarations.push_str(&format!(
            "@group(0) @binding({}) var<storage,{}> ara_particle_{name}:binding_array<{}>;\n",
            200 + r as u32,
            if r.writes() { "read_write" } else { "read" },
            r.ty()
        ));
    }
    let out = rewrite_globals(body, &names, original)?;
    Ok(format!(
        r#"enable wgpu_binding_array;
struct AraRoot {{w0:u32,w1:u32,w2:u32,w3:u32,w4:u32,w5:u32,w6:u32,w7:u32,}};
var<immediate> ara_root:AraRoot;
struct AraParticles {{values:array<TpParticle>,}};
struct AraWords {{values:array<u32>,}};
{declarations}
{out}
"#
    ))
}

// Fail at export when a future name change prevents an alias repair. SPIR-V
// validation alone cannot know Ara's descriptor-set layout.
fn validate_heap_bindings(words: &[u32]) -> Result<()> {
    let mut sets = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        ensure!(
            count > 0 && at + count <= words.len(),
            "invalid SPIR-V instruction"
        );
        if words[at] & 0xffff == 71 && count == 4 {
            if words[at + 2] == 33 {
                bindings.insert(words[at + 1], words[at + 3]);
            }
            if words[at + 2] == 34 {
                sets.insert(words[at + 1], words[at + 3]);
            }
        }
        at += count;
    }
    for (id, binding) in bindings {
        ensure!(
            sets.get(&id) == Some(&0) && [1, 2, 3, 5, 6].contains(&binding),
            "shader resource{id} did not map into Ara's global heap: set{:?} binding{binding}",
            sets.get(&id)
        );
    }
    Ok(())
}

fn mesh_interface(body: &str, original: &naga::Module) -> Result<String> {
    use super::MeshResource as R;
    let mut names = BTreeMap::new();
    let mut declarations = String::new();
    for r in R::ALL {
        let name = r.name();
        let root = r as u32;
        if let Some(ty) = r.buffer_type() {
            names.insert(
                name,
                format!(
                    "ara_mesh_{name}[ara_root.w{root}]{}",
                    if r.array() { ".values" } else { "" }
                ),
            );
            declarations.push_str(&format!(
                "@group(0) @binding({}) var<storage,read> ara_mesh_{name}:binding_array<{ty}>;\n",
                300 + root
            ));
        } else {
            names.insert(
                name,
                format!(
                    "{}[ara_root.w{root}]",
                    if matches!(r, R::BaseSampler | R::MrSampler) {
                        "ara_samplers"
                    } else {
                        "ara_textures"
                    }
                ),
            );
        }
    }
    let out = rewrite_globals(body, &names, original)?;
    Ok(format!(
        r#"enable wgpu_binding_array;
struct AraRoot {{w0:u32,w1:u32,w2:u32,w3:u32,w4:u32,w5:u32,w6:u32,w7:u32,}};
var<immediate> ara_root:AraRoot;
struct AraMatrices {{values:array<mat4x4<f32>>,}};
struct AraNormals {{values:array<NormalTransform>,}};
@group(0) @binding(5) var ara_textures:binding_array<texture_2d<f32>>;
@group(0) @binding(2) var ara_samplers:binding_array<sampler>;
{declarations}
{out}
"#
    ))
}

// Heap declarations precede the emitted body; module directives must precede both.
fn hoist_directives(source: &str) -> String {
    let mut directives = Vec::new();
    let mut body = String::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("enable ")
            || trimmed.starts_with("requires ")
            || trimmed.starts_with("diagnostic(")
        {
            if !directives.contains(&line) {
                directives.push(line);
            }
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    directives.join("\n") + "\n" + &body
}
