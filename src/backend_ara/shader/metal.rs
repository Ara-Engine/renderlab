//! Naga MSL plus a checked entry shim; shader algorithms remain generated code.
use super::{MetalProgram, Pass, parse, validate};
use anyhow::{Context, Result, bail, ensure};
use naga::back::msl;

/// Translate original WGSL and adapt only its entry interface to Ara's global heap.
pub fn msl(pass: Pass) -> Result<MetalProgram> {
    let mut module = parse(pass.source().as_ref())?;
    for (_, global) in module.global_variables.iter_mut() {
        if global.space == naga::AddressSpace::Uniform {
            global.space = naga::AddressSpace::Storage {
                access: naga::StorageAccess::LOAD,
            };
        }
    }
    super::select(&mut module, pass)?;
    let info = validate(&module)?;
    let mut options = msl::Options {
        lang_version: (3, 0),
        fake_missing_bindings: false,
        ..Default::default()
    };
    let mut bindings = msl::BindingMap::default();
    for (_, global) in module.global_variables.iter() {
        if let Some(binding) = global.binding {
            let mut target = msl::BindTarget::default();
            match module.types[global.ty].inner {
                naga::TypeInner::Image { .. } => target.texture = Some(binding.binding as u8),
                naga::TypeInner::Sampler { .. } => {
                    target.sampler = Some(msl::BindSamplerTarget::Resource(0))
                }
                _ => target.buffer = Some(0),
            }
            bindings.insert(binding, target);
        }
    }
    for entry in &module.entry_points {
        options.per_entry_point_map.insert(
            entry.name.clone(),
            msl::EntryPointResources {
                resources: bindings.clone(),
                sizes_buffer: Some(30),
                ..Default::default()
            },
        );
    }
    let (mut source, translation) =
        msl::write_string(&module, &info, &options, &msl::PipelineOptions::default())?;
    let names = translation
        .entry_point_names
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let at = module
        .entry_points
        .iter()
        .position(|e| e.name == pass.entry())
        .context("selected entry not emitted")?;
    source.push_str("\nstruct AraWriteImageEntry {ulong unused; texture2d<float, access::write> texture; ulong metadata;};\nstruct AraWriteVolumeEntry {ulong unused; texture3d<float, access::write> texture; ulong metadata;};\nstruct AraVolumeEntry {ulong unused; texture3d<float> texture; ulong metadata;};\n");
    source.push_str("\nstruct AraDepthEntry {ulong unused; depth2d<float> texture; ulong metadata;};\nstruct AraDepthMsEntry {ulong unused; depth2d_ms<float> texture; ulong metadata;};\n");
    source.push_str("\nstruct AraWriteCubeImageEntry {ulong unused; texture2d_array<float, access::write> texture; ulong metadata;};\n");
    source.push_str("\nstruct AraWriteUintImageEntry {ulong unused; texture2d<uint, access::write> texture; ulong metadata;};\n");
    let entry = wrap(
        &mut source,
        &module,
        &names[at],
        if pass.raster() { "fragment" } else { "kernel" },
        pass,
    )?;
    let vertex_entry = if let Some(i) = module
        .entry_points
        .iter()
        .position(|e| e.stage == naga::ShaderStage::Vertex)
    {
        if matches!(pass, Pass::MeshDiagnostic) {
            Some(wrap(&mut source, &module, &names[i], "vertex", pass)?)
        } else {
            Some(names[i].clone())
        }
    } else {
        None
    };
    if matches!(pass, Pass::MeshDiagnostic) {
        // Ara reserves attributes0..10 for the converter ABI. Explicit streams
        // map WGSL locationN to native attribute11+N on every mesh shader.
        for location in (0..16u32).rev() {
            source = source.replace(
                &format!("[[attribute({location})]]"),
                &format!("[[attribute({})]]", 11 + location),
            );
        }
    }
    Ok(MetalProgram {
        source,
        entry,
        vertex_entry,
    })
}
fn wrap(
    source: &mut String,
    module: &naga::Module,
    name: &str,
    stage: &str,
    pass: Pass,
) -> Result<String> {
    let needle = format!("{name}(");
    let name_at = source.find(&needle).context("entry signature missing")?;
    let start = source[..name_at].rfind('\n').map_or(0, |i| i + 1);
    let open = name_at + name.len();
    let close = matching_paren(source, open)?;
    let prefix = source[start..name_at].to_string();
    let stage_at = prefix
        .find(&format!("{stage} "))
        .context("entry stage qualifier missing")?;
    let result_type = prefix[stage_at + stage.len() + 1..].trim().to_string();
    ensure!(!result_type.is_empty(), "entry return type missing");
    let args = split_args(&source[open + 1..close]);
    let mut plain = Vec::new();
    let mut calls = Vec::new();
    let mut builtins = Vec::new();
    let mut workgroups = Vec::new();
    for arg in args {
        if !arg.contains("[[") && arg.starts_with("threadgroup ") {
            let (ty, name) = arg
                .rsplit_once(' ')
                .context("workgroup argument missing name")?;
            let ty = ty
                .strip_suffix('&')
                .context("workgroup argument must be a reference")?;
            workgroups.push(format!("{ty} {name};"));
            plain.push(arg.into());
            calls.push(name.into());
            continue;
        }
        let (parameter, attribute) = arg
            .split_once("[[")
            .with_context(|| format!("entry argument has no binding: {arg}"))?;
        let parameter = parameter.trim();
        let identifier = parameter
            .split_whitespace()
            .last()
            .context("entry argument missing name")?
            .trim_start_matches('&');
        let (call, builtin) = if identifier == "_buffer_sizes" {
            (String::from("ara_sizes"), None)
        } else if attribute.contains("thread_position_in_grid") {
            (
                "ara_id".to_owned(),
                Some("uint3 ara_id [[thread_position_in_grid]]".to_string()),
            )
        } else if attribute.contains("thread_position_in_threadgroup") {
            (
                "ara_local".to_owned(),
                Some("uint3 ara_local [[thread_position_in_threadgroup]]".to_string()),
            )
        } else if attribute.contains("threadgroup_position_in_grid") {
            (
                "ara_group".to_owned(),
                Some("uint3 ara_group [[threadgroup_position_in_grid]]".to_string()),
            )
        } else if attribute.contains("thread_index_in_threadgroup") {
            (
                "ara_lane".to_owned(),
                Some("uint ara_lane [[thread_index_in_threadgroup]]".to_string()),
            )
        } else if attribute.contains("stage_in") {
            (
                identifier.to_owned(),
                Some(format!("{parameter} [[stage_in]]")),
            )
        } else if attribute.contains("front_facing") {
            (
                "ara_front".into(),
                Some("bool ara_front [[front_facing]]".into()),
            )
        } else if attribute.contains("vertex_id") {
            (
                "ara_vertex".into(),
                Some("uint ara_vertex [[vertex_id]]".into()),
            )
        } else if attribute.contains("instance_id") {
            (
                "ara_instance".into(),
                Some("uint ara_instance [[instance_id]]".into()),
            )
        } else if attribute.contains("position") {
            (
                "ara_position".to_owned(),
                Some("float4 ara_position [[position]]".to_string()),
            )
        } else {
            (resource(identifier, parameter, pass)?, None)
        };
        if let Some(builtin) = builtin {
            builtins.push(builtin);
        }
        plain.push(parameter.to_string());
        calls.push(call);
    }
    let ordinary = format!("{result_type} {name}({})", plain.join(", "));
    source.replace_range(start..close + 1, &ordinary);
    let entry = format!("ara_{name}");
    let mut signature = vec![String::from("constant TopLevel& top [[buffer(2)]]")];
    signature.extend(builtins);
    let signature = signature.join(", ");
    let return_call = if result_type == "void" {
        format!("{name}({});", calls.join(", "))
    } else {
        format!("return {name}({});", calls.join(", "))
    };
    let mut sizes = Vec::new();
    for (_, global) in module.global_variables.iter() {
        if module.types[global.ty]
            .inner
            .needs_host_buffer_byte_size(&module.types)
        {
            let name = global
                .name
                .as_deref()
                .context("runtime buffer missing name")?;
            let slot = if pass.skin() {
                super::SkinResource::from_name(name)
                    .context("skin size buffer mapping missing")?
                    .metal_slot()
            } else {
                format!("top.root[{}]", buffer_root(pass, name)?)
            };
            sizes.push(format!("uint(top.buffers[{slot}].metadata)"));
        }
    }
    let size_init = if calls.iter().any(|v| v == "ara_sizes") {
        format!("_mslBufferSizes ara_sizes = {{{}}};", sizes.join(","))
    } else {
        String::new()
    };
    // Synthetic lengths are thread-local values read from immutable heap metadata,
    // not another native constant-buffer binding. Preserve naga's bounds checks.
    *source = source.replace("constant _mslBufferSizes&", "thread const _mslBufferSizes&");
    source.push_str(&format!(
        "{stage} {result_type} {entry}({signature}) {{ {} {size_init} {return_call} }}\n",
        workgroups.join(" ")
    ));
    Ok(entry)
}

fn resource(name: &str, parameter: &str, pass: Pass) -> Result<String> {
    if matches!(pass, Pass::MeshDiagnostic) {
        use super::MeshResource as R;
        let r = R::from_name(name).with_context(|| format!("unmapped mesh resource{name}"))?;
        let root = r as u32;
        return if r.buffer_type().is_some() {
            buffer_argument(parameter, root)
        } else if matches!(r, R::BaseSampler | R::MrSampler) {
            Ok(format!("top.samplers[top.root[{root}]].state"))
        } else {
            Ok(format!("top.textures[top.root[{root}]].texture"))
        };
    }
    if matches!(pass, Pass::ParticleSort) {
        let resource = super::ParticleResource::from_name(name)
            .with_context(|| format!("unmapped particle resource {name}"))?;
        return buffer_argument(parameter, resource as u32);
    }
    if pass.depth_input() {
        return Ok(match name {
            "scene_depth" | "tp_raster_depth" => format!(
                "((device {}*)top.textures)[top.root[0]].texture",
                if matches!(pass, Pass::RasterSurfaceFour) {
                    "AraDepthMsEntry"
                } else {
                    "AraDepthEntry"
                }
            ),
            "frame" | "tp" => buffer_argument(parameter, 1)?,
            "surface" | "tp_depth_out" => {
                "((device AraWriteImageEntry*)top.storage_images)[top.root[2]].texture".into()
            }
            _ => bail!("unmapped raster surface resource {name}"),
        });
    }
    if pass.skin() {
        let resource = super::SkinResource::from_name(name)
            .with_context(|| format!("unmapped skin resource {name}"))?;
        return buffer_at(parameter, &resource.metal_slot());
    }
    if pass.atmosphere() {
        return Ok(match name {
            "atmosphere" => buffer_argument(parameter, 0)?,
            "atmosphere_transmittance" => "top.textures[top.root[1]].texture".into(),
            "atmosphere_multiscatter" => "top.textures[top.root[2]].texture".into(),
            "at_skyview" | "atmosphere_skyview" => "top.textures[top.root[3]].texture".into(),
            "at_transmittance_out" | "at_multiscatter_out" | "at_skyview_out" => {
                "((device AraWriteImageEntry*)top.storage_images)[top.root[4]].texture".into()
            }
            "at_volume_out" | "at_scatter_out" => {
                "((device AraWriteVolumeEntry*)top.storage_images)[top.root[4]].texture".into()
            }
            "atmosphere_volume" => {
                "((device AraVolumeEntry*)top.textures)[top.root[7]].texture".into()
            }
            "at_history" => "((device AraVolumeEntry*)top.textures)[top.root[6]].texture".into(),
            "at_scatter" => "((device AraVolumeEntry*)top.textures)[top.root[7]].texture".into(),
            "at_specular_out" | "at_diffuse_out" => {
                "((device AraWriteCubeImageEntry*)top.storage_images)[top.root[4]].texture".into()
            }
            "atmosphere_sampler" | "dfg_sampler" => "top.samplers[top.root[5]].state".into(),
            _ => bail!("unmapped atmosphere resource {name}"),
        });
    }
    if matches!(pass, Pass::LightCluster) {
        return Ok(match name {
            "lights" => buffer_argument(parameter, 0)?,
            "frame" => buffer_argument(parameter, 2)?,
            "clusters" => {
                "((device AraWriteUintImageEntry*)top.storage_images)[top.root[1]].texture".into()
            }
            _ => bail!("unmapped light-cluster parameter {name}"),
        });
    }

    Ok(match name {
        "input_colour" | "colour" => "top.textures[top.root[0]].texture".into(),
        "second_colour" | "bloom" => "top.textures[top.root[1]].texture".into(),
        "auxiliary" => "top.textures[top.root[2]].texture".into(),
        "velocity_tiles" => "top.textures[top.root[3]].texture".into(),
        "output_colour" => {
            "((device AraWriteImageEntry*)top.storage_images)[top.root[4]].texture".into()
        }
        "settings" => "*reinterpret_cast<device Settings*>(top.buffers[top.root[5]].data)".into(),
        "linear_sampler" => "top.samplers[top.root[6]].state".into(),
        "lut" => "((device AraVolumeEntry*)top.textures)[top.root[7]].texture".into(),
        _ => bail!("unmapped naga entry parameter {name}"),
    })
}
fn matching_paren(source: &str, open: usize) -> Result<usize> {
    let mut depth = 0;
    for (offset, c) in source[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(open + offset);
                }
            }
            _ => {}
        }
    }
    bail!("unterminated entry signature")
}
fn split_args(args: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    for (i, c) in args.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                output.push(args[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    if !args[start..].trim().is_empty() {
        output.push(args[start..].trim());
    }
    output
}

fn buffer_argument(parameter: &str, root: u32) -> Result<String> {
    buffer_at(parameter, &format!("top.root[{root}]"))
}
fn buffer_at(parameter: &str, slot: &str) -> Result<String> {
    let (ty, _) = parameter
        .rsplit_once(' ')
        .context("buffer parameter type missing")?;
    let ty = ty.trim();
    ensure!(
        ty.contains("device "),
        "expected storage parameter: {parameter}"
    );
    if let Some(value) = ty.strip_suffix('&') {
        Ok(format!(
            "*reinterpret_cast<{}*>(top.buffers[{slot}].data)",
            value.trim()
        ))
    } else if ty.ends_with('*') {
        Ok(format!("reinterpret_cast<{ty}>(top.buffers[{slot}].data)"))
    } else {
        bail!("unexpected storage parameter {parameter}")
    }
}
fn buffer_root(pass: Pass, name: &str) -> Result<u32> {
    if matches!(pass, Pass::MeshDiagnostic) {
        return Ok(super::MeshResource::from_name(name).context("mesh runtime buffer")? as u32);
    }
    if matches!(pass, Pass::ParticleSort) {
        return Ok(
            super::ParticleResource::from_name(name).context("particle runtime buffer")? as u32,
        );
    }
    match (pass, name) {
        (Pass::LightCluster, "lights") => Ok(0),
        (Pass::LightCluster, "frame") => Ok(2),
        (_, "settings") => Ok(5),
        _ => bail!("unmapped runtime buffer {name}"),
    }
}
