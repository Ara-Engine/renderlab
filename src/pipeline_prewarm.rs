//! Load-time preparation of the scene-dependent variants found by the cut census.
use super::*;

impl Renderer {
    /// Pipelines compiled since the last successful prepare (`Engine::prepare_pipelines`),
    /// each one a lazy first-use compile a frame waited for; also logged as
    /// `pipeline_compile_after_prepare` and in the pipeline log's frame events. The goal
    /// is 0. Never counts before the first prepare.
    pub fn compiles_after_prepare(&self) -> u64 {
        self.device.compiles_after_prepare()
    }

    /// The selected view's own pipelines (its look and render scale applied): post, the
    /// visibility buffer's and, for traced shading, every program in `programs` (the loaded
    /// opaque programs) as a frame drawing only it and, with several, as a frame drawing all
    /// of them would build: shading sets, list pipelines and binning, reflection hits and
    /// lighting-cache cell shading. Called by `Engine::prepare_pipelines` for each view
    /// before `prepare_scene_pipelines`.
    pub(crate) fn prepare_view_pipelines(&mut self, programs: &[u64]) -> Result<(), Error> {
        self.check_fault()?;
        if self.view.post_settings.enabled {
            self.post_pipelines()?;
        }
        if self.primary_mode == PrimaryVisibility::VisibilityBuffer && self.view.primary.is_some() {
            self.ensure_visibility_pipelines()?;
        }
        // Traced shading's passes, in the shading variant a lit frame of this view picks.
        let traced = self.primary_mode != PrimaryVisibility::Raster
            && self.view.primary.is_some()
            && self.primary_sources.is_some();
        if traced {
            let paths = self.frame_paths(ViewMode::Lit, self.view.shadows.enabled(ViewMode::Lit), true);
            if !paths.temporal {
                let shading = if paths.soft {
                    primary_ray_gpu::Shading::Soft
                } else {
                    primary_ray_gpu::Shading::Hard
                };
                if let Some(variant) = paths.variant {
                    self.ensure_pipelines(programs.iter().map(|&id| (id, variant)).collect())?;
                }
                if programs.len() > 1 {
                    self.ensure_lists(programs, false, shading as usize)?;
                    self.ensure_bins()?;
                    self.ensure_reflection_hits_for(programs, true, shading)?;
                }
                self.ensure_reflection_trace(shading)?;
                self.ensure_reflection_hits_for(programs, false, shading)?;
                self.prepare_gi_shade(programs)?;
                {
                    use primary_ray_gpu::gi_receiver_pass as receiver;
                    if receiver::enabled(&self.device) {
                        let key = self.gi_receiver_key(shading, receiver::reuse());
                        self.gi_receiver_pipelines(shading, key)?;
                    }
                }
                // Temporal reconstruction, as `render_frame` turns it on for a lit frame.
                if self.view.reconstruction.enabled
                    && !self.view.outputs.modes.data()
                    && self.view.soft_mode != crate::SoftShadows::Temporal
                    && self.pipeline.primary.is_some()
                {
                    self.ensure_reconstruction()?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn prepare_scene_pipelines(
        &mut self,
        materials: &[&Material],
    ) -> Result<crate::engine::PipelinePreparation, Error> {
        self.check_fault()?;
        if materials
            .iter()
            .any(|m| !Arc::ptr_eq(&self.identity, &m.renderer))
        {
            return Err(Error::InvalidInput("material belongs to another renderer"));
        }
        let start = Instant::now();
        let before = self.stats();
        let scopes = OperationScopes::new(&self.device);
        let prepared = self.prewarm_transparency(materials).and_then(|()| {
            // The lighting cache's kernels, on the path its frames will trace with.
            if self.gi.settings.enabled {
                let software = self.hardware_shadows.is_none() || self.gi.settings.force_software;
                self.ensure_gi_pipelines(software)?;
            }
            // Sky, aerial perspective and fog.
            if self.atmosphere.settings.active() {
                self.ensure_atmosphere_kernels()?;
            }
            Ok(())
        });
        if prepared.is_ok()
            && self.hardware_shadows.is_none()
            && self.local_shadows().count > 0
        {
            self.local_maps.prewarm_clear(&self.device, &self.ledger);
        }
        let error = scopes.finish();
        prepared?;
        if let Some(error) = error {
            return Err(match error {
                wgpu::Error::Validation { .. } => Error::Shader(error.to_string()),
                _ => self.fail(error.to_string()),
            });
        }
        let after = self.stats();
        self.device.mark_prepared();
        Ok(crate::engine::PipelinePreparation {
            cpu: start.elapsed(),
            render_pipelines: after.ownership.get(ResourceKind::RenderPipeline).created
                - before.ownership.get(ResourceKind::RenderPipeline).created,
            compute_pipelines: after.ownership.get(ResourceKind::ComputePipeline).created
                - before.ownership.get(ResourceKind::ComputePipeline).created,
            added_buffer_bytes: after
                .hal
                .buffer_bytes
                .saturating_sub(before.hal.buffer_bytes)
                .max(0) as u64,
            added_texture_bytes: after
                .hal
                .texture_bytes
                .saturating_sub(before.hal.texture_bytes)
                .max(0) as u64,
        })
    }
}
