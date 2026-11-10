//! Original lit toon program on a single indexed raster batch and heap input packets.
use super::{
    bindings::HeapBindings,
    cache::ShaderCache,
    graph::{Graph, ResourceUse},
    image::Texture,
    mesh::{MeshGeometry, mesh_state},
    post_surface::RasterDepth,
    shader::Pass,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource,
    commands::{Attachment, Command, Recorder},
    descriptors::Format,
    texture::SampleCount,
};
/// One material/program packet and its instanced indexed geometry.
pub struct ToonDraw<'a, 'resources> {
    /// Borrowed geometry streams, slot records and model table.
    pub geometry: &'a MeshGeometry,
    /// Complete selected logical interface, resolved into one persistent heap record.
    pub bindings: &'a HeapBindings<'resources>,
}
/// Backed single-sample raster targets and explicit load/clear intent.
pub struct MeshAttachments<'a> {
    /// Color attachment matching the pipeline format.
    pub color: &'a Texture,
    /// Reversed Depth32Float attachment.
    pub depth: &'a RasterDepth,
    /// None loads a previous producer, such as the sky backdrop.
    pub clear_color: Option<[f32; 4]>,
    /// None preserves depth for a subsequent opaque batch.
    pub clear_depth: Option<f32>,
}
/// Persistent original vs/fs_lit pipeline, using Metal4 per-vertex interpolation.
pub struct ToonPass {
    pipeline: Pipeline,
    format: Format,
}
impl ToonPass {
    /// Compile the production composer and original authored toon program.
    /// Vulkan requires Ara's pending fragment-shader-barycentric feature enablement.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, format: Format) -> Result<Self> {
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::MeshToon, mesh_state(format))?,
            format,
        })
    }
    /// Declare the union before rendering, then record all materials in one native
    /// raster scope. Optional effects keep their real initialized input resources.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        draws: &[ToonDraw<'_, '_>],
        targets: &MeshAttachments<'_>,
    ) -> Result<()> {
        ensure!(
            !draws.is_empty() && draws.iter().all(|d| d.bindings.pass() == Pass::MeshToon),
            "empty/incompatible toon batch"
        );
        ensure!(
            targets.color.format == self.format
                && targets.color.size == targets.depth.size()
                && targets.color.shape.samples == SampleCount::One
                && targets.depth.samples() == SampleCount::One,
            "toon attachment mismatch"
        );
        if targets.clear_color.is_none() {
            graph.require_initialized(Resource::Image(targets.color.image))?;
        }
        if targets.clear_depth.is_none() {
            graph.require_initialized(Resource::Image(targets.depth.image()))?;
        }
        let mut uses = vec![
            ResourceUse::Whole(Resource::Image(targets.color.image), Access::Color),
            ResourceUse::Whole(Resource::Image(targets.depth.image()), Access::Depth),
        ];
        for draw in draws {
            uses.extend(draw.bindings.uses());
            uses.extend(draw.geometry.raster_uses().map(ResourceUse::from));
        }
        graph.node_uses(rec, &uses)?;
        rec.push(Command::BeginPass("ara::ToonOpaque".into()));
        rec.push(Command::BeginRendering {
            colors: vec![Attachment {
                image: targets.color.image,
                clear: targets.clear_color,
            }],
            depth: Some(Attachment {
                image: targets.depth.image(),
                clear: targets.clear_depth.map(|d| [d, 0., 0., 0.]),
            }),
            size: targets.color.size,
        });
        rec.push(Command::Pipeline(self.pipeline));
        for draw in draws {
            rec.push(Command::Constants(draw.bindings.roots()));
            draw.geometry.draw(rec);
        }
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Retire after the last submitted raster batch.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        gpu.retire_pipeline(self.pipeline)?;
        Ok(())
    }
}
