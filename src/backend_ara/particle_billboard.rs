//! Original sorted particle billboards with premultiplied scene composition.
use super::{
    bindings::HeapBindings,
    cache::ShaderCache,
    graph::{Graph, ResourceUse},
    image::Texture,
    shader::{InputKey, Pass},
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource,
    commands::{Attachment, Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
    texture::{SampleCount, ViewDimension},
};
/// One sorted emitter; preserve back-to-front order across this slice.
pub struct BillboardDraw<'a, 'resources> {
    /// Original frame/emitter/particle/key/sprite/lighting/atmosphere inputs.
    pub inputs: &'a HeapBindings<'resources>,
    /// Number of sorted ranks, bounded by the key buffer and original2048 slots.
    pub instances: u32,
}
/// Persistent original six-vertex, instanced billboard program.
pub struct ParticleBillboard {
    pipeline: Pipeline,
    format: Format,
}
impl ParticleBillboard {
    /// Compile both unchanged shader stages with premultiplied source-over blending.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, format: Format) -> Result<Self> {
        let state = RenderState {
            color_formats: vec![format.vk().as_raw()],
            blend: true,
            ..Default::default()
        };
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::ParticleBillboard, state)?,
            format,
        })
    }
    /// Record an ordered emitter batch in one raster scope, loading a previous HDR
    /// producer when clear is None. All inputs are declared before rendering starts.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        draws: &[BillboardDraw<'_, '_>],
        target: &Texture,
        clear: Option<[f32; 4]>,
    ) -> Result<()> {
        ensure!(
            !draws.is_empty()
                && target.format == self.format
                && target.view.dimension == ViewDimension::D2
                && target.shape.samples == SampleCount::One,
            "invalid particle billboard target/batch"
        );
        let mut uses = vec![ResourceUse::Whole(
            Resource::Image(target.image),
            Access::Color,
        )];
        for draw in draws {
            ensure!(
                draw.inputs.pass() == Pass::ParticleBillboard
                    && (1..=2048).contains(&draw.instances),
                "invalid particle billboard packet/count"
            );
            let keys = draw
                .inputs
                .buffer(InputKey {
                    group: 0,
                    binding: 41,
                })
                .ok_or_else(|| anyhow::anyhow!("particle key input missing"))?;
            ensure!(
                keys.len >= u64::from(draw.instances) * 4,
                "particle key buffer is too short"
            );
            uses.extend(draw.inputs.uses());
        }
        if clear.is_none() {
            graph.require_initialized(Resource::Image(target.image))?;
        }
        graph.node_uses(rec, &uses)?;
        rec.push(Command::BeginPass("ara::ParticleBillboard".into()));
        rec.push(Command::BeginRendering {
            colors: vec![Attachment {
                image: target.image,
                clear,
            }],
            depth: None,
            size: target.size,
        });
        rec.push(Command::Pipeline(self.pipeline));
        for draw in draws {
            rec.push(Command::Constants(draw.inputs.roots()));
            rec.push(Command::DrawProcedural {
                vertices: 6,
                instances: draw.instances,
            });
        }
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Nothing of its own to release: its pipelines stay with the shader cache
    /// (`ShaderCache::retire`).
    pub fn retire(self, _: &mut GpuDevice) -> Result<()> {
        Ok(())
    }
}
