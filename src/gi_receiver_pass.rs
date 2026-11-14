//! Optional geometry-only GI receiver preparation, before either shading path and its lists.
use super::*;
use crate::program::{self, Stage};
pub(crate) const BINDING: u32 = 68;
const BYTES: u64 = 48;
const SOURCE: &str = include_str!("../shaders/gi_receiver_pass.wgsl");
const ENTRY_SEAM: &str = "// ---- GI receiver compute entry ----";
/// Defaults are approved separately for each path after game-scene picture and memory checks.
pub(crate) const RT_DEFAULT: bool = false;
pub(crate) const NON_RT_DEFAULT: bool = false;
const HEAD_MARK: &str = "\n// renderlab: this renderer prepares GI receivers before shading\n";
fn env_switch(value: Option<&str>) -> Option<bool> {
    match value {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => None,
    }
}
fn requested() -> Option<bool> {
    static ON: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *ON.get_or_init(|| env_switch(std::env::var("RENDERLAB_GI_PASS").ok().as_deref()))
}
/// Explicit opt-in only. The port uses this for its existing refusal and never adopts
/// the wgpu renderer's path defaults.
pub(crate) fn on() -> bool {
    requested() == Some(true)
}
fn path_default(hardware: bool) -> bool {
    if hardware { RT_DEFAULT } else { NON_RT_DEFAULT }
}
fn default_on(device: &wgpu::Device) -> bool {
    path_default(
        device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
    )
}
pub(super) fn enabled(device: &wgpu::Device) -> bool {
    requested().unwrap_or_else(|| default_on(device))
}
fn default_head_for(head: &mut String, default: bool, override_on: Option<bool>) {
    if default && override_on != Some(false) {
        head.push_str(HEAD_MARK);
    }
}
/// Carry this renderer's default through every program composed on its shading heads.
/// Unmarked heads (including the port) retain inline lookup unless explicitly enabled.
pub(super) fn default_head(head: &mut String, device: &wgpu::Device) {
    default_head_for(head, default_on(device), requested());
}
fn source_enabled(source: &str, override_on: Option<bool>) -> bool {
    override_on.unwrap_or_else(|| source.contains(HEAD_MARK))
}
pub(crate) fn for_source(source: &str) -> bool {
    source_enabled(source, requested())
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
    let enabled = for_source(&source);
    shade_source_for(source, eligible, specular, enabled)
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
        if !enabled(&self.device) {
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
        if !enabled(&self.device) {
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
    fn path_defaults_and_overrides_agree_with_composition() {
        for default in [false, true] {
            for (value, expected) in [
                (None, default),
                (Some(""), default),
                (Some("0"), false),
                (Some("1"), true),
                (Some("other"), default),
            ] {
                let requested = env_switch(value);
                let mut head = "unmodified head".to_owned();
                default_head_for(&mut head, default, requested);
                assert_eq!(source_enabled(&head, requested), expected);
                assert_eq!(requested.unwrap_or(default), expected);
                if !default || requested == Some(false) {
                    assert_eq!(head, "unmodified head");
                }
            }
        }
        // A renderer's default mark never enables an unrelated, unmarked backend head.
        assert!(!source_enabled("port head", None));
    }
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
            let record = module
                .types
                .iter()
                .find(|(_, t)| t.name.as_deref() == Some("GiPrepared"))
                .unwrap()
                .1;
            let naga::TypeInner::Struct { members, span } = &record.inner else {
                panic!("GI record must be a struct")
            };
            assert_eq!(*span, BYTES as u32);
            assert_eq!(
                members.iter().map(|m| m.offset).collect::<Vec<_>>(),
                [0, 16, 32]
            );
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
