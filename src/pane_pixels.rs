//! Per-pixel pane shading, shared by both backends (shaders/pane_pixels.wgsl): the glass
//! module's `fs_transparent` rewritten as `tp_pane_fragment`, a function of explicit
//! derivatives a compute stage can call, and the sections of the pane entries. The ara-gpu
//! port (src/backend_ara/transparency_glass.rs) and the wgpu renderer
//! (src/transparency_gpu.rs) compose their pane modules from these.

/// The pane entries (`PANE_COMMON`, `PANE_RASTER`, `PANE_COMPUTE`).
const PANE: &str = include_str!("../shaders/pane_pixels.wgsl");

/// One `// ---- name ----` section of shaders/pane_pixels.wgsl.
pub(crate) fn section(name: &str) -> Result<&'static str, String> {
    let start = format!("// ---- {name} ----");
    let end = format!("// ---- end {name} ----");
    let from = PANE
        .find(&start)
        .ok_or_else(|| format!("pane_pixels.wgsl has no {name}"))?;
    let to = PANE[from..]
        .find(&end)
        .ok_or_else(|| format!("pane_pixels.wgsl does not end {name}"))?;
    Ok(&PANE[from..from + to + end.len()])
}

/// The raster API's implicit-derivative samples rewritten for a compute stage: the material
/// samples with the input's own uv derivatives, the one-level split-sum table at its level.
/// Only inside those API functions: other entries of the module keep their own sampling.
pub(crate) fn explicit_sampling(source: &str) -> Result<String, String> {
    let mut source = source.to_owned();
    for (function, from, to) in [
        (
            "fn sample_base_color(input:SurfaceInput)",
            "textureSample(base_color,base_sampler,input.uv)",
            "textureSampleGrad(base_color,base_sampler,input.uv,input.uv_dx,input.uv_dy)",
        ),
        (
            "fn sample_metallic_roughness(input:SurfaceInput)",
            "textureSample(metallic_roughness,mr_sampler,input.uv)",
            "textureSampleGrad(metallic_roughness,mr_sampler,input.uv,input.uv_dx,input.uv_dy)",
        ),
        (
            "fn sample_normal_map(input:SurfaceInput)",
            "textureSample(normal_map,normal_sampler,input.uv,1)",
            "textureSampleGrad(normal_map,normal_sampler,input.uv,1,input.uv_dx,input.uv_dy)",
        ),
        (
            "fn sample_normal_map(input:SurfaceInput)",
            "textureSample(normal_map,normal_sampler,input.uv,0)",
            "textureSampleGrad(normal_map,normal_sampler,input.uv,0,input.uv_dx,input.uv_dy)",
        ),
        // The split-sum table has one level: its level 0 is what the raster samples.
        (
            "fn environment_dfg(n_dot_v:f32, roughness:f32)",
            "textureSample(dfg,dfg_sampler,uv)",
            "textureSampleLevel(dfg,dfg_sampler,uv,0.0)",
        ),
    ] {
        let at = source
            .find(function)
            .ok_or_else(|| format!("the raster API has no {function}"))?;
        let span = function_span(&source[at..])?;
        let body = source[at..at + span].to_owned();
        if !body.contains(from) {
            return Err(format!("{function} no longer samples {from}"));
        }
        source.replace_range(at..at + span, &body.replace(from, to));
    }
    Ok(source)
}

/// The length of the function `text` starts with, through its closing brace.
fn function_span(text: &str) -> Result<usize, String> {
    let open = text.find('{').ok_or("a function without a body")?;
    let mut depth = 0usize;
    for (at, ch) in text[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(open + at + 1);
                }
            }
            _ => {}
        }
    }
    Err("a function that does not close".to_owned())
}

/// `fs_transparent` as `tp_pane_fragment(input, triangle, front, d)`: the same body, its
/// derivatives taken from `d` (`PaneDerivatives`) and a discard returned as alpha -1. Taken
/// from the composed glass module, so it follows the shared entry as it changes.
pub(crate) fn fragment(glass: &str) -> Result<String, String> {
    let head = "@fragment fn fs_transparent(input:RasterInput, triangle:RasterTriangle, @builtin(front_facing) front:bool) -> @location(0) vec4<f32> {";
    let start = glass
        .find(head)
        .ok_or("the glass module's fs_transparent changed its signature")?;
    let body = &glass[start + head.len() - 1..];
    let span = function_span(body)?;
    let mut body = body[1..span - 1].to_owned();
    for (from, to) in [
        ("dpdx(input.world)", "d.world_x"),
        ("dpdy(input.world)", "d.world_y"),
        ("dpdx(input.normal)", "d.normal_x"),
        ("dpdy(input.normal)", "d.normal_y"),
        ("dpdx(input.uv)", "d.uv_x"),
        ("dpdy(input.uv)", "d.uv_y"),
        ("discard;", "return vec4<f32>(-1.0);"),
    ] {
        body = body.replace(from, to);
    }
    if body.contains("dpdx") || body.contains("dpdy") || body.contains("discard") {
        return Err(
            "fs_transparent differentiates or discards something the pane shading does not supply"
                .to_owned(),
        );
    }
    Ok(format!(
        "fn tp_pane_fragment(input:RasterInput, triangle:RasterTriangle, front:bool, d:PaneDerivatives) -> vec4<f32> {{{body}}}"
    ))
}
