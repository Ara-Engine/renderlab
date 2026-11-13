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

/// H's dispatch arguments, after its count word (rl_hit_args).
const HIT_ARGS_OFFSET: u64 = 4;

impl Renderer {
    /// Each frame program's hit pipeline for `shading` (single-program or list ABI), compiled
    /// the first time a frame with the reflection pass shades with it.
    pub(crate) fn ensure_reflection_hits(&mut self, shading: Shading) -> Result<(), Error> {
        let Some(state) = self.view.primary.as_ref() else {
            return Ok(());
        };
        if !reflection_pass::on(self.device.features()) || state.requests {
            return Ok(());
        }
        let mixed = state.mixed();
        let ids = if mixed {
            state.programs.clone()
        } else {
            vec![state.program]
        };
        let shading = shading as usize;
        for id in ids {
            if self.program_slot(id).pipelines.hits[usize::from(mixed)][shading].is_some() {
                continue;
            }
            let head = &self.primary_sources.as_ref().unwrap().1[shading];
            let slot = self.program_slot(id);
            let parameters = program::declares_parameters(&slot.source);
            let composed =
                program::compose_hits(head, &slot.source, Stage::Traced, parameters, mixed);
            let abi = self.program_abi();
            let scopes = OperationScopes::new(&self.device);
            let module = super::program_gpu::module(&abi, &slot.label, &composed);
            let state = self.view.primary.as_ref().unwrap();
            let layout = if mixed {
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
        if !reflection_pass::on(self.device.features()) || state.requests {
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
        // Each dispatch: its program's pipeline and group 0 with the slot's dynamic offset.
        let dispatches: Option<Vec<(&wgpu::ComputePipeline, &wgpu::BindGroup, u32)>> = if mixed {
            let group = bindings.list[shading as usize].as_ref();
            let stride = state.list_stride();
            state
                .programs
                .iter()
                .enumerate()
                .map(|(k, &id)| Some((&**hits(id)?, &**group?, k as u32 * stride?)))
                .collect()
        } else {
            let group = bindings.shade[shading as usize].as_ref();
            hits(state.program)
                .zip(group)
                .map(|(pipeline, group)| vec![(&**pipeline, &**group, 0)])
        };
        let Some(dispatches) = dispatches else {
            return;
        };
        let g = state.geometry.as_ref().unwrap();
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("reflection hits"),
            timestamp_writes: None,
        });
        compute.set_bind_group(1, &*g.arrays, &[]);
        compute.set_bind_group(2, &**g.materials.as_ref().unwrap(), &[]);
        compute.set_bind_group(3, self.lighting.bind(), &[]);
        for (pipeline, group, offset) in dispatches {
            compute.set_pipeline(pipeline);
            if mixed {
                compute.set_bind_group(0, group, &[offset]);
            } else {
                compute.set_bind_group(0, group, &[]);
            }
            compute.dispatch_workgroups_indirect(&pass.hits.value, HIT_ARGS_OFFSET);
        }
    }
}
