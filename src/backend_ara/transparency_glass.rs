//! Original programmable transparent pane shading into seeded coverage samples.
use super::shader::memo::{self, Memo};
use super::{
    bindings::HeapBindings,
    cache::ShaderCache,
    graph::Graph,
    mesh::{MeshGeometry, mesh_state},
    shader::{InputKey, Pass, Program},
    transparency_seed::MsaaColor,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource,
    commands::{Attachment, Command, Recorder},
    descriptors::Format,
    texture::SampleCount,
};
/// Compose the production non-RT transparent material, preserving its authored hooks.
pub fn program(material: &str) -> Result<Program> {
    Ok(Program::new(Pass::TransparencyGlass, source(material))?.checked())
}
/// The same material on a ray-traced renderer's head: the pane's sun visibility is a ray
/// its own fragment traces against the frame's scene, not a lookup in the sun map.
pub fn traced_program(material: &str) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(material), || traced_program_uncached(material))
}
fn traced_program_uncached(material: &str) -> Result<Program> {
    Ok(Program::new(Pass::TransparencyGlass, traced_source(material)?)?.checked())
}
/// The glass module on the ray-traced head (`traced_program`'s source).
fn traced_source(material: &str) -> Result<String> {
    let head = crate::hardware_shadows::shader_source(
        include_str!("../../shaders/scene.wgsl"),
        crate::SampleCount::One,
    )
    .map_err(|e| anyhow::anyhow!("hardware head: {e}"))?;
    Ok(crate::renderer::transparency_gpu::glass_source(
        &head,
        true,
        material,
        crate::program::declares_parameters(material),
    ))
}

/// The per-pixel pane entries appended to the glass module (`shaders/pane_pixels.wgsl`).
const PANE: &str = include_str!("shaders/pane_pixels.wgsl");
fn pane_section(name: &str) -> Result<&'static str> {
    let start = format!("// ---- {name} ----");
    let end = format!("// ---- end {name} ----");
    let from = PANE
        .find(&start)
        .ok_or_else(|| anyhow::anyhow!("pane_pixels.wgsl has no {name}"))?;
    let to = PANE[from..]
        .find(&end)
        .ok_or_else(|| anyhow::anyhow!("pane_pixels.wgsl does not end {name}"))?;
    Ok(&PANE[from..from + to + end.len()])
}
/// The traced glass material's pane id raster: which of the pane's triangles covers each
/// of a pixel's four samples (`tp_pane_id_vs`, `tp_pane_id_fs`).
pub fn pane_id_program(material: &str) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(material), || {
        Ok(Program::new(Pass::PaneId, pane_id_source(material)?)?.checked())
    })
}
pub(crate) fn pane_id_source(material: &str) -> Result<String> {
    // `tp_pane_id_fs` reads the triangle's index, an extension WGSL enables up front.
    Ok(format!(
        "enable primitive_index;\n{}\n{}\n{}\n",
        traced_source(material)?,
        pane_section("PANE_COMMON")?,
        pane_section("PANE_RASTER")?
    ))
}
/// The traced glass material's per-pixel pane shading (`tp_pane_shade`): `fs_transparent`'s
/// own body as a function of explicit derivatives, on a compute stage.
pub fn pane_shade_program(material: &str) -> Result<Program> {
    static KEPT: Memo<[u8; 32], Program> = Memo::new();
    KEPT.get(memo::digest(material), || {
        Ok(Program::new(Pass::PaneShade, pane_shade_source(material)?)?.checked())
    })
}
pub(crate) fn pane_shade_source(material: &str) -> Result<String> {
    let glass = explicit_sampling(&traced_source(material)?)?;
    Ok(format!(
        "{glass}\n{}\n{}\n{}\n",
        pane_fragment(&glass)?,
        pane_section("PANE_COMMON")?,
        pane_section("PANE_COMPUTE")?
    ))
}
/// The raster API's implicit-derivative samples rewritten for a compute stage: the material
/// samples with the input's own uv derivatives, the one-level split-sum table at its level.
/// Only inside those API functions: other entries of the module keep their own sampling.
fn explicit_sampling(source: &str) -> Result<String> {
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
            .ok_or_else(|| anyhow::anyhow!("the raster API has no {function}"))?;
        let span = function_span(&source[at..])?;
        let body = source[at..at + span].to_owned();
        ensure!(body.contains(from), "{function} no longer samples {from}");
        source.replace_range(at..at + span, &body.replace(from, to));
    }
    Ok(source)
}
/// The length of the function `text` starts with, through its closing brace.
fn function_span(text: &str) -> Result<usize> {
    let open = text
        .find('{')
        .ok_or_else(|| anyhow::anyhow!("a function without a body"))?;
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
    anyhow::bail!("a function that does not close")
}
/// `fs_transparent` as `tp_pane_fragment(input, triangle, front, d)`: the same body, its
/// derivatives taken from `d` (`PaneDerivatives`) and a discard returned as alpha -1. Taken
/// from the composed glass module, so it follows the shared entry as it changes.
fn pane_fragment(glass: &str) -> Result<String> {
    let head = "@fragment fn fs_transparent(input:RasterInput, triangle:RasterTriangle, @builtin(front_facing) front:bool) -> @location(0) vec4<f32> {";
    let start = glass.find(head).ok_or_else(|| {
        anyhow::anyhow!("the glass module's fs_transparent changed its signature")
    })?;
    let body = &glass[start + head.len()..];
    let mut depth = 1usize;
    let mut end = None;
    for (at, ch) in body.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(at);
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &body[..end.ok_or_else(|| anyhow::anyhow!("fs_transparent does not close"))?];
    let mut body = body.to_owned();
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
    ensure!(
        !body.contains("dpdx") && !body.contains("dpdy") && !body.contains("discard"),
        "fs_transparent differentiates or discards something the pane shading does not supply"
    );
    Ok(format!(
        "fn tp_pane_fragment(input:RasterInput, triangle:RasterTriangle, front:bool, d:PaneDerivatives) -> vec4<f32> {{{body}}}"
    ))
}
pub(crate) fn source(material: &str) -> String {
    crate::renderer::transparency_gpu::glass_source(
        include_str!("../../shaders/scene.wgsl"),
        false,
        material,
        crate::program::declares_parameters(material),
    )
}
/// Persistent four-sample original vs/fs_transparent pipeline. Vulkan needs the
/// same fragment-barycentric capability as the opaque programmable raster entry.
pub struct Glass {
    pipeline: Pipeline,
    contract: [u8; 32],
}
impl Glass {
    /// Compile caller-authored material WGSL with depth disabled, as on Renderlab.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, program: &Program) -> Result<Self> {
        ensure!(
            program.pass() == Pass::TransparencyGlass,
            "wrong transparent program family"
        );
        let mut state = mesh_state(Format::Rgba16Float);
        state.depth_format = None;
        state.depth_test = false;
        state.depth_write = false;
        state.blend = false;
        state.raster.samples = SampleCount::Four;
        Ok(Self {
            pipeline: cache.program_state(gpu, program, state)?,
            contract: program.contract(),
        })
    }
    /// Draw one sorted pane, loading coverage samples seeded by Seed. Resolve the
    /// current color into the packet's behind image before another overlapping pane.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        geometry: &MeshGeometry,
        target: &MsaaColor,
    ) -> Result<()> {
        ensure!(
            inputs.pass() == Pass::TransparencyGlass && inputs.contract() == self.contract,
            "incompatible transparent packet"
        );
        for binding in [31, 32] {
            let image = inputs
                .image(InputKey { group: 0, binding })
                .ok_or_else(|| anyhow::anyhow!("transparent color/depth input missing"))?;
            ensure!(
                image.size == target.size(),
                "transparent color/depth viewport mismatch"
            );
        }
        graph.require_initialized(Resource::Image(target.image()))?;
        let mut uses = inputs.uses();
        uses.extend(geometry.raster_uses().map(super::graph::ResourceUse::from));
        uses.push((Resource::Image(target.image()), Access::Color).into());
        graph.node_uses(rec, &uses)?;
        rec.push(Command::BeginPass("ara::TransparentPane".into()));
        rec.push(Command::BeginRendering {
            colors: vec![Attachment {
                image: target.image(),
                clear: None,
            }],
            depth: None,
            size: target.size(),
        });
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(inputs.roots()));
        geometry.draw(rec);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
}

/// The pane id raster's pipeline (`pane_id_program`): one sample's triangle ids into an
/// R32Uint target, one sample (no multisampling), no depth, no blending.
pub struct PaneIds {
    pipeline: Pipeline,
    contract: [u8; 32],
}
impl PaneIds {
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, program: &Program) -> Result<Self> {
        ensure!(
            program.pass() == Pass::PaneId,
            "wrong pane id program family"
        );
        let mut state = mesh_state(Format::R32Uint);
        state.depth_format = None;
        state.depth_test = false;
        state.depth_write = false;
        state.blend = false;
        state.raster.samples = SampleCount::One;
        Ok(Self {
            pipeline: cache.program_state(gpu, program, state)?,
            contract: program.contract(),
        })
    }
    /// Draws one pane into `target` (R32Uint at the frame's size), cleared to 0 first.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        geometry: &MeshGeometry,
        target: &super::image::Texture,
    ) -> Result<()> {
        ensure!(
            inputs.pass() == Pass::PaneId && inputs.contract() == self.contract,
            "incompatible pane id packet"
        );
        let mut uses = inputs.uses();
        uses.extend(geometry.raster_uses().map(super::graph::ResourceUse::from));
        uses.push((Resource::Image(target.image), Access::Color).into());
        graph.node_uses(rec, &uses)?;
        rec.push(Command::BeginPass("ara::PaneIds".into()));
        rec.push(Command::BeginRendering {
            colors: vec![Attachment {
                image: target.image,
                clear: Some([0.; 4]),
            }],
            depth: None,
            size: target.size,
        });
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(inputs.roots()));
        geometry.draw(rec);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pane entries compose with the standard program on the traced head, validate, and
    /// reflect what the frame binds: the id raster its sample record, the shading its
    /// pane geometry, the four id images and the output.
    #[test]
    fn pane_programs_compose_validate_and_reflect_their_inputs() {
        let ids = pane_id_program(crate::STANDARD_PROGRAM).unwrap();
        let shade = pane_shade_program(crate::STANDARD_PROGRAM).unwrap();
        let bound = |p: &Program, binding: u32| {
            p.inputs()
                .iter()
                .any(|i| i.key.group == 0 && i.key.binding == binding)
        };
        assert!(bound(&ids, 77) && bound(&ids, 32));
        for binding in 70..=77 {
            assert!(bound(&shade, binding), "the shading binds {binding}");
        }
    }
}
