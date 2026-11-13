//! Coherent local-light visibility before material shading. Child of primary_ray_gpu.
use super::*;
use crate::program::{self, Stage};
pub(super) const BINDING: u32 = 73;
pub(super) const PIXEL_BYTES: u64 = 16;
#[derive(Clone)]
pub(super) struct Pipeline {
    head: String,
    pipeline: Owned<wgpu::ComputePipeline>,
}
const API: &str = include_str!("../shaders/light_vis_pass.wgsl");
const LOCAL: &str = "    if lights[s.index].kind.w!=0u { return rl_local_shadow(";
const PREPARE: &str = "    rl_lights_prepare(surface_input.world);";
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let requested=std::env::var("RENDERLAB_LIGHT_VIS_PASS").is_ok_and(|v|v=="1");
        // The bound needs the material's unshadowed contribution and running radiance.
        // Until t3/material-pass publishes base/metal/rough/normal at bindings 75/78,
        // leave visibility inline whenever the bound is set. That record is the seam
        // for evaluating the shared bound before tracing here, without a second BRDF.
        if requested && std::env::var_os("RENDERLAB_LIGHT_BOUND").is_some() {
            eprintln!("light_vis_pass bypassed: RENDERLAB_LIGHT_BOUND is set; visibility stays inline until material-record integration (75/78)");
            return false;
        }
        requested
    })
}
pub(super) fn layout_entry() -> wgpu::BindGroupLayoutEntry {
    buffer(
        BINDING,
        wgpu::BufferBindingType::Storage { read_only: false },
        4,
    )
}
/// One composition seam; the disabled branch returns the original module untouched.
pub(crate) fn apply(mut composed: program::Composed, traced: bool) -> program::Composed {
    if !traced || !on() || composed.source.contains("struct FilmFrame") {
        return composed;
    }
    let marker = "// ---- program ----\n";
    let before = composed.source.find(marker).unwrap();
    composed.source = entry_enabled(compose_enabled(composed.source, true), true);
    let delta = composed.source.find(marker).unwrap() - before;
    composed.program = (composed.program.start + delta)..(composed.program.end + delta);
    composed
}
fn compose_enabled(mut source: String, enabled: bool) -> String {
    if !enabled {
        return source;
    }
    assert_eq!(source.matches(LOCAL).count(), 1);
    source = source.replacen(
        LOCAL,
        &format!(
            "    let cached=rl_lvis_lookup(input,s); if cached>=0.0 {{ return cached; }}\n{LOCAL}"
        ),
        1,
    );
    // The entry is appended separately, after its reconstruction helpers.
    source.push_str(API);
    source
}
fn entry_enabled(mut source: String, enabled: bool) -> String {
    if !enabled {
        return source;
    }
    // Set only after surface/reflection work has finished; secondary hits never read it.
    source = source.replacen(PREPARE, &format!("{PREPARE}\n    rl_lvis_ready=true;"), 1);
    let reconstruction = include_str!("../shaders/program_traced_entry.wgsl");
    let begin = reconstruction.find("fn primary_shade_pixel(").unwrap();
    let end = reconstruction[begin..]
        .find("    rl_indirect_prepare(surface_input);")
        .unwrap()
        + begin;
    let mut pixel =
        reconstruction[begin..end].replace("fn primary_shade_pixel(", "fn rl_lvis_pixel(");
    pixel=pixel.replace("if record.x==0u {textureStore(primary_hdr,pixel,primary_miss(id,size,primary_frame.clear));return;}","if record.x==0u {return;}");
    pixel.push_str("    rl_lvis_store(surface_input);\n}\n");
    source.push_str(&pixel);
    source
}
impl Renderer {
    pub(crate) fn ensure_light_vis(&mut self, shading: Shading) -> Result<(), Error> {
        if !on() {
            return Ok(());
        }
        let state = self.view.primary.as_ref().unwrap();
        let head = &self.primary_sources.as_ref().unwrap().1[shading as usize];
        if state.light_vis[shading as usize]
            .as_ref()
            .is_some_and(|p| p.head == *head)
        {
            return Ok(());
        }
        // Engine-only entry: no user surface, BRDF, textures, GI or reflection work.
        let source = "struct Surface { unused:f32 }; fn surface(input:SurfaceInput)->Surface {return Surface(0.0);} fn shade(s:Surface,input:SurfaceInput)->vec3<f32> {return vec3<f32>(0.0);}";
        let composed = program::compose(head, source, Stage::Traced, false);
        let abi = self.program_abi();
        let scopes = OperationScopes::new(&self.device);
        let module = super::super::program_gpu::module(&abi, "local light visibility", &composed);
        let started = std::time::Instant::now();
        let pipeline = self.ledger.own(
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("local light visibility"),
                    layout: Some(&state.shade_layouts[shading as usize]),
                    module: &module,
                    entry_point: Some("light_vis_tiles"),
                    compilation_options: Default::default(),
                    cache: abi.cache,
                }),
            ResourceKind::ComputePipeline,
        );
        if let Some(error) = scopes.finish() {
            return Err(super::super::program_gpu::scope_error(
                "local light visibility",
                error,
                &[composed],
            ));
        }
        eprintln!(
            "light_vis_compile seconds={:.3}",
            started.elapsed().as_secs_f64()
        );
        self.view.primary.as_mut().unwrap().light_vis[shading as usize] = Some(Pipeline {
            head: head.clone(),
            pipeline,
        });
        Ok(())
    }
    pub(crate) fn encode_light_vis(&self, encoder: &mut wgpu::CommandEncoder, shading: Shading) {
        if !on() {
            return;
        }
        let state = self.view.primary.as_ref().unwrap();
        let g = state.geometry.as_ref().unwrap();
        let bindings = state.bindings.as_ref().unwrap();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("local light visibility"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&state.light_vis[shading as usize].as_ref().unwrap().pipeline);
        pass.set_bind_group(
            0,
            &**bindings.shade[shading as usize].as_ref().unwrap(),
            &[],
        );
        pass.set_bind_group(1, &*g.arrays, &[]);
        pass.set_bind_group(2, &**g.materials.as_ref().unwrap(), &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        let [x, y] = state.region_groups([8, 4]);
        pass.dispatch_workgroups(x, y, 1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_paths_validate_with_cache() {
        for heads in [
            shading_heads(include_str!("../shaders/scene.wgsl")).unwrap(),
            non_rt_sources(include_str!("../shaders/scene.wgsl"))
                .unwrap()
                .1,
        ] {
            for head in heads {
                let c = program::compose(&head, crate::STANDARD_PROGRAM, Stage::Traced, false);
                let source = entry_enabled(compose_enabled(c.source, true), true);
                let module = naga::front::wgsl::parse_str(&source)
                    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .unwrap();
            }
        }
    }
    #[test]
    fn off_is_byte_identical_and_on_has_one_seam() {
        let input = format!("header\n{LOCAL}light,input.receiver,s.direction); }}");
        assert_eq!(compose_enabled(input.clone(), false), input);
        let on = compose_enabled(input, true);
        assert_eq!(on.matches(LOCAL).count(), 1);
        assert_eq!(on.matches("let cached=rl_lvis_lookup").count(), 1);
        assert_eq!(PIXEL_BYTES, 16);
    }
}
