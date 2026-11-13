//! Optional geometry-only GI receiver preparation, before either shading path and its lists.
use super::*;
use crate::program::{self, Stage};
pub(crate) const BINDING: u32 = 68;
const BYTES: u64 = 48;
const SOURCE: &str = include_str!("../shaders/gi_receiver_pass.wgsl");
const ENTRY_SEAM: &str = "// ---- GI receiver compute entry ----";
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_PASS").is_ok_and(|s| s == "1"))
}
pub(crate) fn entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: BINDING,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(BYTES),
        },
        count: None,
    }
}
/// The GI receiver kernel's module on a traced shading `head`, exactly as the engine compiles
/// it: the standard program's traced composition (which carries the shading-side definitions
/// `shade_source` appends) and the kernel half of gi_receiver_pass.wgsl.
pub(crate) fn kernel_module(head: &str) -> program::Composed {
    let mut composed = program::compose(head, program::STANDARD_PROGRAM, Stage::Traced, false);
    composed
        .source
        .push_str(SOURCE.split_once(ENTRY_SEAM).unwrap().1);
    composed
}
pub(crate) fn shade_source(source: String, eligible: bool, specular: bool) -> String {
    shade_source_for(source, eligible, specular, on())
}
fn shade_source_for(mut source: String, eligible: bool, specular: bool, enabled: bool) -> String {
    if !enabled || !eligible {
        return source;
    }
    source = source.replace(
        "    rl_indirect_prepare(surface_input);",
        if specular {
            "    gi_pass_load(surface_input,id,size); es_prepare(surface_input);"
        } else {
            "    gi_pass_load(surface_input,id,size);"
        },
    );
    source.push_str(SOURCE.split_once(ENTRY_SEAM).unwrap().0);
    source
}
pub(super) struct Targets {
    pub values: Buffer,
    args: Buffer,
    pipelines: [Option<Owned<wgpu::ComputePipeline>>; 2],
    source_hash: [[u8; 32]; 2],
}
impl Renderer {
    pub(super) fn gi_receiver_targets(&self, size: wgpu::Extent3d) -> Targets {
        Targets {
            values: Buffer::new(
                self,
                "GI prepared receiver",
                u64::from(size.width) * u64::from(size.height) * BYTES,
                wgpu::BufferUsages::STORAGE,
            ),
            args: Buffer::new(
                self,
                "GI receiver dispatch",
                12,
                wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            ),
            pipelines: [None, None],
            source_hash: [[0; 32]; 2],
        }
    }
    pub(crate) fn ensure_gi_receiver_pass(&mut self, shading: Shading) -> Result<(), Error> {
        if !on() {
            return Ok(());
        }
        let Some(state) = self.view.primary.as_ref() else {
            return Ok(());
        };
        let Some(target) = state.targets.as_ref().and_then(|t| t.gi_receiver.as_ref()) else {
            return Ok(());
        };
        let [x, y] = state.region_groups([8, 4]);
        self.queue.write_buffer(
            &target.args.value,
            0,
            &[x, y, 1u32]
                .into_iter()
                .flat_map(u32::to_ne_bytes)
                .collect::<Vec<_>>(),
        );
        if target.pipelines[shading as usize].is_some()
            && target.source_hash[shading as usize] == self.primary_hash
        {
            return Ok(());
        }
        let head = &self.primary_sources.as_ref().unwrap().1[shading as usize];
        let composed = kernel_module(head);
        let abi = self.program_abi();
        let scopes = OperationScopes::new(&self.device);
        let module = super::program_gpu::module(&abi, "GI receiver", &composed);
        let start = std::time::Instant::now();
        let pipeline = self.ledger.own(
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("GI receiver"),
                    layout: Some(&state.shade_layouts[shading as usize]),
                    module: &module,
                    entry_point: Some("gi_receiver_pass"),
                    compilation_options: Default::default(),
                    cache: abi.cache,
                }),
            ResourceKind::ComputePipeline,
        );
        eprintln!(
            "gi_receiver_compile_seconds={:.3}",
            start.elapsed().as_secs_f64()
        );
        if let Some(error) = scopes.finish() {
            return Err(super::program_gpu::scope_error(
                "GI receiver",
                error,
                &[composed],
            ));
        }
        self.pipelines.created += 1;
        self.view
            .primary
            .as_mut()
            .unwrap()
            .targets
            .as_mut()
            .unwrap()
            .gi_receiver
            .as_mut()
            .unwrap()
            .source_hash[shading as usize] = self.primary_hash;
        self.view
            .primary
            .as_mut()
            .unwrap()
            .targets
            .as_mut()
            .unwrap()
            .gi_receiver
            .as_mut()
            .unwrap()
            .pipelines[shading as usize] = Some(pipeline);
        Ok(())
    }
    pub(crate) fn encode_gi_receiver_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        shading: Shading,
    ) {
        if !on() {
            return;
        }
        let state = self.view.primary.as_ref().unwrap();
        let Some(target) = state.targets.as_ref().and_then(|t| t.gi_receiver.as_ref()) else {
            return;
        };
        let Some(pipeline) = &target.pipelines[shading as usize] else {
            return;
        };
        let bindings = state.bindings.as_ref().unwrap();
        let g = state.geometry.as_ref().unwrap();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("GI receiver"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(
            0,
            &**bindings.shade[shading as usize].as_ref().unwrap(),
            &[],
        );
        pass.set_bind_group(1, &*g.arrays, &[]);
        pass.set_bind_group(2, &**g.materials.as_ref().unwrap(), &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        pass.dispatch_workgroups_indirect(&target.args.value, 0);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_composition_is_byte_identical() {
        for eligible in [false, true] {
            for specular in [false, true] {
                let source = "    rl_indirect_prepare(surface_input);\n".to_owned();
                assert_eq!(
                    shade_source_for(source.clone(), eligible, specular, false),
                    source
                );
            }
        }
        assert_eq!(BYTES, 48);
    }
    #[test]
    fn receiver_and_shading_validate_both_paths() {
        let prelude = include_str!("../shaders/scene.wgsl");
        let hardware = super::super::shading_sources(prelude, program::STANDARD_PROGRAM).unwrap();
        let (_, software_heads) = super::super::non_rt_sources(prelude).unwrap();
        let software = software_heads.map(|head| {
            program::compose(&head, program::STANDARD_PROGRAM, Stage::Traced, false).source
        });
        for source in hardware.into_iter().chain(software) {
            let mut source = if source.contains("struct GiPrepared") {
                source
            } else {
                shade_source_for(source, true, true, true)
            };
            source.push_str(SOURCE.split_once(ENTRY_SEAM).unwrap().1);
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
