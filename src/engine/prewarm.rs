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

    /// Prepare's goal: once a scene is prepared, no frame compiles a pipeline, from any view
    /// of it. Three programs (the standard floor, a second opaque program, glass) and an emitter
    /// created after prepare (as a game spawns one in play), then views where each program is
    /// in turn the frame's first, one with all of them, and a cut back.
    #[test]
    #[ignore = "GPU: compiles after prepare, both paths"]
    fn a_prepared_scene_compiles_nothing_in_its_frames() {
        use super::super::view_tests::cube;
        use crate::engine::{Camera, ExperimentalRayQuery, MeshData, ViewDesc};
        use glam::{Mat4, Vec3};
        let adapter = super::super::test_adapter();
        for path in [RenderPath::NonRt, RenderPath::HardwareRt] {
            let mut engine = Engine::new(
                &adapter,
                EngineDescriptor {
                    path,
                    // SAFETY: the test suite's documented experimental-feature acknowledgement.
                    experimental_ray_query: (path == RenderPath::HardwareRt)
                        .then(|| unsafe { ExperimentalRayQuery::acknowledge() }),
                    ..Default::default()
                },
            )
            .unwrap();
            let second = engine
                .create_program("second", &format!("{}\n// second\n", crate::STANDARD_PROGRAM))
                .unwrap();
            let spawn = |engine: &mut Engine, c, h, desc: MaterialDesc| {
                let (vertices, indices) = cube(c, h);
                let mesh = engine.create_mesh(&MeshData { vertices: &vertices, indices: &indices }).unwrap();
                let material = engine.create_material(&desc).unwrap();
                engine.spawn(mesh, material, Mat4::IDENTITY).unwrap();
            };
            spawn(&mut engine, [0.0, -0.05, 0.0], [20.0, 0.05, 20.0], MaterialDesc::default());
            spawn(&mut engine, [-6.0, 1.0, 0.0], [1.0, 1.0, 1.0], MaterialDesc {
                program: Some(second),
                base_color: [0.8, 0.3, 0.2, 1.0],
                ..Default::default()
            });
            spawn(&mut engine, [6.0, 1.0, 0.0], [1.0, 1.0, 1.0], MaterialDesc {
                transparency: Some(crate::Transparency::default()),
                ..Default::default()
            });
            // A soft sun as Ara's scenes have: without ray hardware its shadows are the virtual
            // clipmap's, and reflection hits read its map.
            engine.set_sun(Some(crate::engine::Sun::default())).unwrap();
            let view = engine.create_view(&ViewDesc::new(160, 96)).unwrap();
            engine.prepare_pipelines().unwrap();
            // A game spawns its emitters once play starts, after the scene is prepared.
            engine
                .create_emitter(
                    &crate::EmitterDescriptor {
                        position: Vec3::new(0.0, 1.0, 0.0),
                        burst: 64,
                        size: [0.3, 0.3],
                        ..Default::default()
                    },
                    None,
                )
                .unwrap();
            let look = |eye: [f32; 3], at: [f32; 3]| {
                Camera::look_at(Vec3::from(eye), Vec3::from(at), Vec3::Y, 0.9, 0.05, 200.0).unwrap()
            };
            let cameras = [
                look([0.0, 12.0, 16.0], [0.0, 0.0, 0.0]),   // everything
                look([0.0, 6.0, 0.5], [0.0, 0.0, 0.0]),     // the floor alone
                look([-6.0, 1.5, 3.0], [-6.0, 1.0, 0.0]),   // the second program first
                look([6.0, 1.5, 3.0], [6.0, 1.0, 0.0]),     // glass over the floor
                look([0.0, 12.0, 16.0], [0.0, 0.0, 0.0]),   // a cut back
            ];
            for camera in &cameras {
                for _ in 0..4 {
                    engine.tick(1.0 / 60.0).unwrap();
                    engine.render_view(view, camera).unwrap();
                }
            }
            let late = engine.renderer().compiles_after_prepare();
            // Each one is logged as `pipeline_compile_after_prepare` with its label.
            assert_eq!(late, 0, "{path:?}: {late} pipelines compiled after prepare");
        }
    }
}

impl Engine {
    /// Makes the loaded scene ready to draw: creates the pipelines its frames would otherwise
    /// compile on first use. So far: every view's post pipelines, transparent material
    /// variants for every loaded material (visible or not), the lighting cache's and the
    /// atmosphere's kernels and the local-shadow atlas clear. Call after creating views and
    /// loading materials and settings, and again after adding a level or replacing programs.
    /// No frame is rendered, histories and particles do not advance, and no camera or
    /// output-sized target is needed.
    ///
    /// Pipelines frames still compile afterwards are counted (`Renderer::compiles_after_prepare`)
    /// and logged as `pipeline_compile_after_prepare`; the goal is none. Not a guarantee against
    /// driver first-use work or arbitrary future settings. The default-off `RENDERLAB_PREWARM_PIPELINES=1` calls it automatically
    /// before the engine's first frame. Explicit calls do not need that environment flag.
    /// It compiles up to four pipelines at once (`RENDERLAB_PREPARE_COMPILES`); frames keep
    /// the process's base limit.
    pub fn prepare_pipelines(&mut self) -> Result<PipelinePreparation, Error> {
        crate::pipeline_compilation::with_compile_slots(
            crate::pipeline_compilation::prepare_compile_slots(),
            || self.prepare_pipelines_now(),
        )
    }

    fn prepare_pipelines_now(&mut self) -> Result<PipelinePreparation, Error> {
        use crate::ResourceKind;
        // The opaque programs a frame can draw: one entry per loaded program, not per material.
        let mut programs: Vec<u64> = self
            .state
            .materials
            .ids()
            .map(|id| &self.state.materials.get(id).unwrap().material)
            .filter(|m| m.transparency.is_none())
            .map(|m| m.program.id())
            .collect();
        programs.sort_unstable();
        programs.dedup();
        // Each view's own pipelines first, with its look applied as its frames apply it.
        let before = self.renderer.stats().ownership;
        for at in 0..self.views.len() {
            self.renderer.select_view(self.views[at].id)?;
            self.apply_look(at)?;
            self.renderer.prepare_view_pipelines(&programs)?;
        }
        let after = self.renderer.stats().ownership;
        let made = |kind| after.get(kind).created - before.get(kind).created;
        let (view_render, view_compute) = (made(ResourceKind::RenderPipeline), made(ResourceKind::ComputePipeline));
        let materials: Vec<_> = self
            .state
            .materials
            .ids()
            .map(|id| &self.state.materials.get(id).unwrap().material)
            .collect();
        let mut report = self.renderer.prepare_scene_pipelines(&materials)?;
        report.render_pipelines += view_render;
        report.compute_pipelines += view_compute;
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
