//! The Mac thread's reflection hit shading on wgpu (docs/design-reflection-pass.md), after
//! U-RTVIS's `rl_trace` and `rl_hit_args` (reflection_pass.rs). Each hit record in H is shaded
//! by its program's hit module (`program::compose_hits`: the traced module with the
//! `reflection_hits` entry, shaders/reflection_hits.wgsl) in an indirect dispatch whose
//! arguments `rl_hit_args` wrote, and its radiance lands in R at the record's slot. The hit
//! entry runs on the traced shading ABI: H (64) and R (65) join the shading group while
//! `RENDERLAB_RL_PASS=1` (crate::renderer::reflection_hits::shade_entries).
//!
//! A single-program frame shades all of H with its program in one dispatch. A mixed-program
//! frame dispatches each of its programs over all of H on MP1's list ABI (its slot uniform at
//! a dynamic offset), each keeping the hits on its own draws; H keeps rl_trace's tile order,
//! so a group's records are mostly one program's. Frames that stream textures bind another
//! materials group and skip the hits for now (R keeps rl_trace's clear at hit slots).
//! A child of primary_ray_gpu (its state and layouts).
use super::*;
use crate::program::{self, Stage};
#[path = "reflection_hit_sun.rs"]
mod hit_sun;

/// Measurement only (`RENDERLAB_RL_HIT_SUN_COUNT=1`, read with `RENDERLAB_RL_ROOM_LOG`, whose
/// `floored_inline` pair then holds them): the hits that ask the map for the sun (word 7) and
/// those it cannot answer, which trace their ray (word 8). The counters are then copied after
/// the hit pass instead of after the trace (reflection_pass.rs `rl_room_copy`).
pub(super) fn hit_sun_count() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_RL_HIT_SUN_COUNT").is_ok_and(|v| v == "1"))
}
const SUN_COUNT_SEAM: &str = "        let v=rl_hit_sun_map(receiver,l);\n";
const SUN_COUNT: &str = "        let v=rl_hit_sun_map(receiver,l);\n        atomicAdd(&rl_room_stats[7],1u); if v<0.0 { atomicAdd(&rl_room_stats[8],1u); }\n";
const SUN_COUNT_BINDING: &str =
    "\n@group(0) @binding(69) var<storage,read_write> rl_room_stats:array<atomic<u32>,16>;\n";

/// H's dispatch arguments, after its count word (rl_hit_args).
const HIT_ARGS_OFFSET: u64 = 4;

impl Renderer {
    /// Each frame program's hit pipeline for `shading` (single-program or list ABI), compiled
    /// the first time a frame with the reflection pass shades with it.
    pub(crate) fn ensure_reflection_hits(&mut self, shading: Shading) -> Result<(), Error> {
        let Some(state) = self.view.primary.as_ref() else {
            return Ok(());
        };
        if !reflection_pass::on(&self.device) || state.requests {
            return Ok(());
        }
        let mixed = state.mixed();
        let ids = if mixed {
            state.programs.clone()
        } else {
            vec![state.program]
        };
        let shading = shading as usize;
        let map = self.hit_sun_buffers().is_some();
        for id in ids {
            let pipelines = &self.program_slot(id).pipelines;
            if pipelines.hits[usize::from(mixed)][shading].is_some()
                && pipelines.hits_sun[usize::from(mixed)][shading] == map
            {
                continue;
            }
            let head = &self.primary_sources.as_ref().unwrap().1[shading];
            let slot = self.program_slot(id);
            let parameters = program::declares_parameters(&slot.source);
            let mut composed =
                program::compose_hits(head, &slot.source, Stage::Traced, parameters, mixed);
            // The sun from the map (reflection_hit_sun.rs): groups 0-3 as the program's shading
            // pipeline holds them (its layout's, as lean_gpu.rs takes them), then the map's.
            let shading_pipeline = if mixed {
                slot.pipelines.list[0][shading].as_ref()
            } else {
                slot.pipelines.traced[shading].as_ref()
            };
            let mut sun_layout = None;
            if let (true, Some(full)) = (map, shading_pipeline)
                && let Some(source) = hit_sun::compose(composed.source.clone())
            {
                composed.source = source;
                if hit_sun_count() && composed.source.contains(SUN_COUNT_SEAM) {
                    composed.source = composed.source.replacen(SUN_COUNT_SEAM, SUN_COUNT, 1);
                    if !composed
                        .source
                        .contains("var<storage,read_write> rl_room_stats")
                    {
                        composed.source.push_str(SUN_COUNT_BINDING);
                    }
                }
                let groups: [_; 4] = std::array::from_fn(|g| full.get_bind_group_layout(g as u32));
                let own = hit_sun::layout(&self.device);
                sun_layout = Some(
                    self.ledger.own(
                        self.device
                            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                                label: Some("reflection hits with the sun map"),
                                bind_group_layouts: &[
                                    Some(&groups[0]),
                                    Some(&groups[1]),
                                    Some(&groups[2]),
                                    Some(&groups[3]),
                                    Some(&own),
                                ],
                                immediate_size: 0,
                            }),
                        ResourceKind::PipelineLayout,
                    ),
                );
            }
            let sun = sun_layout.is_some();
            let abi = self.program_abi();
            let scopes = OperationScopes::new(&self.device);
            let module = super::program_gpu::module(&abi, &slot.label, &composed);
            let state = self.view.primary.as_ref().unwrap();
            let layout = if let Some(layout) = sun_layout.as_deref() {
                layout
            } else if mixed {
                state.list_layout(shading)
            } else {
                &state.shade_layouts[shading]
            };
            let pipeline = self.ledger.own(
                self.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("reflection hits"),
                        layout: Some(layout),
                        module: &module,
                        entry_point: Some("reflection_hits"),
                        compilation_options: Default::default(),
                        cache: abi.cache,
                    }),
                ResourceKind::ComputePipeline,
            );
            if let Some(error) = scopes.finish() {
                return Err(
                    match super::program_gpu::scope_error(&slot.label, error, &[composed]) {
                        Error::Fault(Fault(message)) => self.fail(message),
                        other => other,
                    },
                );
            }
            self.pipelines.created += 1;
            let slot = self.programs.iter_mut().find(|s| s.id == id).unwrap();
            slot.pipelines.hits[usize::from(mixed)][shading] = Some(pipeline);
            slot.pipelines.hits_sun[usize::from(mixed)][shading] = sun;
        }
        Ok(())
    }

    /// The hit dispatches over H with `rl_hit_args`'s arguments, after `rl_trace`: one per
    /// frame program. Nothing while the pass is off, in streaming frames, or before the
    /// pipelines and groups exist.
    pub(crate) fn encode_reflection_hits(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        shading: Shading,
    ) {
        let Some(state) = self.view.primary.as_ref() else {
            return;
        };
        if !reflection_pass::on(&self.device) || state.requests {
            return;
        }
        let (Some(pass), Some(bindings)) = (
            state.targets.as_ref().and_then(|t| t.rl_pass.as_ref()),
            state.bindings.as_ref(),
        ) else {
            return;
        };
        let mixed = state.mixed();
        let hits = |id: u64| {
            self.program_slot(id).pipelines.hits[usize::from(mixed)][shading as usize].as_ref()
        };
        // The map group for pipelines that read it (reflection_hit_sun.rs), made this frame
        // from the map's current buffers.
        let sun_buffers = self.hit_sun_buffers();
        let sun_group = |pipeline: &wgpu::ComputePipeline| {
            let buffers = sun_buffers?;
            Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("reflection hits sun map"),
                layout: &pipeline.get_bind_group_layout(hit_sun::GROUP),
                entries: &[0u32, 1, 2].map(|binding| wgpu::BindGroupEntry {
                    binding,
                    resource: buffers[binding as usize].as_entire_binding(),
                }),
            }))
        };
        let uses_sun = |id: u64| {
            self.program_slot(id).pipelines.hits_sun[usize::from(mixed)][shading as usize]
        };
        // Each dispatch: its program's pipeline and group 0 with the slot's dynamic offset.
        let dispatches: Option<Vec<(&wgpu::ComputePipeline, &wgpu::BindGroup, u32, bool)>> =
            if mixed {
                let group = bindings.list[shading as usize].as_ref();
                let stride = state.list_stride();
                state
                    .programs
                    .iter()
                    .enumerate()
                    .map(|(k, &id)| {
                        Some((&**hits(id)?, &**group?, k as u32 * stride?, uses_sun(id)))
                    })
                    .collect()
            } else {
                let group = bindings.shade[shading as usize].as_ref();
                hits(state.program).zip(group).map(|(pipeline, group)| {
                    vec![(&**pipeline, &**group, 0, uses_sun(state.program))]
                })
            };
        let Some(dispatches) = dispatches else {
            return;
        };
        // A pipeline built for the map needs its group; without the buffers this frame (the map
        // went away) the hits wait for ensure to rebuild it.
        let mut suns = Vec::with_capacity(dispatches.len());
        for (pipeline, _, _, sun) in &dispatches {
            let group = if *sun { sun_group(pipeline) } else { None };
            if *sun && group.is_none() {
                return;
            }
            suns.push(group);
        }
        let g = state.geometry.as_ref().unwrap();
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("reflection hits"),
            timestamp_writes: None,
        });
        compute.set_bind_group(1, &*g.arrays, &[]);
        compute.set_bind_group(2, &**g.materials.as_ref().unwrap(), &[]);
        compute.set_bind_group(3, self.lighting.bind(), &[]);
        for ((pipeline, group, offset, _), sun) in dispatches.into_iter().zip(&suns) {
            compute.set_pipeline(pipeline);
            if let Some(sun) = sun {
                compute.set_bind_group(hit_sun::GROUP, sun, &[]);
            }
            if mixed {
                compute.set_bind_group(0, group, &[offset]);
            } else {
                compute.set_bind_group(0, group, &[]);
            }
            compute.dispatch_workgroups_indirect(&pass.hits.value, HIT_ARGS_OFFSET);
        }
        drop(compute);
        if hit_sun_count() {
            self.rl_room_copy(encoder, pass);
        }
    }
}
