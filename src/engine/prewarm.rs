//! Explicit preparation at the end of a level load, before its first visible frame.
use super::Engine;
use crate::Error;
use std::time::Duration;

/// CPU preparation and additional owned GPU allocation, excluding driver residency.
#[derive(Clone, Copy, Debug, Default)]
pub struct PipelinePreparation {
    pub cpu: Duration,
    pub render_pipelines: u64,
    pub compute_pipelines: u64,
    pub added_buffer_bytes: u64,
    pub added_texture_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{EngineDescriptor, MaterialDesc, RenderPath, Settings};

    #[test]
    #[ignore = "GPU: loaded invisible material and idempotent preparation, both paths"]
    fn loaded_materials_prepare_without_a_view_or_object() {
        let adapter = super::super::test_adapter();
        for path in [RenderPath::NonRt, RenderPath::HardwareRt] {
            let mut engine = Engine::new(
                &adapter,
                EngineDescriptor {
                    path,
                    experimental_ray_query: (path == RenderPath::HardwareRt)
                        .then(|| unsafe { crate::engine::ExperimentalRayQuery::acknowledge() }),
                    settings: Settings {
                        global_illumination: false,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
            engine
                .create_material(&MaterialDesc {
                    transparency: Some(crate::Transparency::default()),
                    ..Default::default()
                })
                .unwrap();
            let frames = engine.renderer().stats().frames;
            let first = engine.prepare_pipelines().unwrap();
            assert!(first.render_pipelines >= 8, "{path:?}: {first:?}");
            assert!(first.compute_pipelines >= 2, "{path:?}: {first:?}");
            let second = engine.prepare_pipelines().unwrap();
            assert_eq!((second.render_pipelines, second.compute_pipelines), (0, 0));
            assert_eq!(
                (second.added_buffer_bytes, second.added_texture_bytes),
                (0, 0)
            );
            assert_eq!(engine.renderer().stats().frames, frames);
        }
    }
}

impl Engine {
    /// Creates transparent material variants for every loaded material, visible or not,
    /// and the local-shadow atlas clear. Call after loading materials and settings, and
    /// again after adding a level or replacing programs. No frame is rendered, histories
    /// and particles do not advance, and no camera or output-sized target is needed.
    ///
    /// This synchronous call covers the scene-dependent variants in the forest cut
    /// census; it is not a guarantee against driver first-use work or arbitrary future
    /// settings. The default-off `RENDERLAB_PREWARM_PIPELINES=1` calls it automatically
    /// before the engine's first frame. Explicit calls do not need that environment flag.
    pub fn prepare_pipelines(&mut self) -> Result<PipelinePreparation, Error> {
        let materials: Vec<_> = self
            .state
            .materials
            .ids()
            .map(|id| &self.state.materials.get(id).unwrap().material)
            .collect();
        let report = self.renderer.prepare_scene_pipelines(&materials)?;
        self.prewarm_pending = false;
        eprintln!(
            "pipeline_prepare ms={:.3} render={} compute={} buffer_bytes={} texture_bytes={}",
            report.cpu.as_secs_f64() * 1e3,
            report.render_pipelines,
            report.compute_pipelines,
            report.added_buffer_bytes,
            report.added_texture_bytes
        );
        Ok(report)
    }
}
