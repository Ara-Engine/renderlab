//! The Mac thread's reflection hit shading on wgpu (docs/design-reflection-pass.md), after
//! U-RTVIS's `rl_trace` and `rl_hit_args` (reflection_pass.rs). Each hit record in H is shaded
//! by its program's hit module (`program::compose_hits`: the traced module with the
//! `reflection_hits` entry, shaders/reflection_hits.wgsl) in one indirect dispatch whose
//! arguments `rl_hit_args` wrote, and its radiance lands in R at the record's slot. The hit
//! entry runs on the traced shading ABI: H (64) and R (65) join the shading group while
//! `RENDERLAB_RL_PASS=1` (crate::renderer::reflection_hits::shade_entries).
//!
//! First stage: single-program frames. A mixed-program frame's hits stay unshaded (R keeps
//! rl_trace's clear) until the binning kernel (shaders/reflection_hit_bins.wgsl) dispatches
//! each program over its own records. A child of primary_ray_gpu (its state and layouts).
use super::*;
use crate::program::{self, Stage};

/// H's dispatch arguments, after its count word (rl_hit_args).
const HIT_ARGS_OFFSET: u64 = 4;

impl Renderer {
    /// The frame program's hit pipeline for `shading`, compiled the first time a frame with
    /// the reflection pass shades with it. Reloads drop it with the traced set.
    pub(crate) fn ensure_reflection_hits(&mut self, shading: Shading) -> Result<(), Error> {
        let Some(state) = self.view.primary.as_ref() else {
            return Ok(());
        };
        if !reflection_pass::on() || state.mixed() {
            return Ok(());
        }
        let id = state.program;
        let shading = shading as usize;
        if self.program_slot(id).pipelines.hits[shading].is_some() {
            return Ok(());
        }
        let head = &self.primary_sources.as_ref().unwrap().1[shading];
        let slot = self.program_slot(id);
        let parameters = program::declares_parameters(&slot.source);
        let composed = program::compose_hits(head, &slot.source, Stage::Traced, parameters);
        let abi = self.program_abi();
        let scopes = OperationScopes::new(&self.device);
        let module = super::program_gpu::module(&abi, &slot.label, &composed);
        let layout = &self.view.primary.as_ref().unwrap().shade_layouts[shading];
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
        slot.pipelines.hits[shading] = Some(pipeline);
        Ok(())
    }

    /// The hit dispatch over H with `rl_hit_args`'s arguments, after `rl_trace`; nothing while
    /// the pass is off, in mixed-program frames, or before the pipeline exists.
    pub(crate) fn encode_reflection_hits(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        shading: Shading,
    ) {
        let Some(state) = self.view.primary.as_ref() else {
            return;
        };
        if !reflection_pass::on() || state.mixed() {
            return;
        }
        let (Some(pipeline), Some(pass), Some(group)) = (
            self.program_slot(state.program).pipelines.hits[shading as usize].as_ref(),
            state.targets.as_ref().and_then(|t| t.rl_pass.as_ref()),
            state
                .bindings
                .as_ref()
                .and_then(|b| b.shade[shading as usize].as_ref()),
        ) else {
            return;
        };
        let g = state.geometry.as_ref().unwrap();
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("reflection hits"),
            timestamp_writes: None,
        });
        compute.set_pipeline(pipeline);
        compute.set_bind_group(0, &**group, &[]);
        compute.set_bind_group(1, &*g.arrays, &[]);
        compute.set_bind_group(2, &**g.materials.as_ref().unwrap(), &[]);
        compute.set_bind_group(3, self.lighting.bind(), &[]);
        compute.dispatch_workgroups_indirect(&pass.hits.value, HIT_ARGS_OFFSET);
    }
}
