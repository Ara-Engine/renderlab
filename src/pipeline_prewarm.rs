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
        let prepared = self.prewarm_transparency(materials);
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
