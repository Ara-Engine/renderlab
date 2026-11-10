//! Original hardware cluster-ID raster; software clusters use the shared Kernel.
use super::{
    bindings::HeapBindings,
    buffer::StorageBuffer,
    cache::ShaderCache,
    graph::{Graph, ResourceUse},
    post_surface::RasterDepth,
    shader::{InputKey, Pass},
};
use anyhow::{Context, Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource,
    commands::{Attachment, Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
    texture::SampleCount,
};
/// Retained source tables and their total cluster count, independent of visible count.
pub struct HardwareClusters<'a, 'resources> {
    /// Original config, cluster/model/normal/placement/list and data-array inputs.
    pub inputs: &'a HeapBindings<'resources>,
    /// Config.info.x and the starting word of the hardware list.
    pub count: u32,
}
/// Persistent procedural cluster draw into the shared packed-u64 visibility buffer.
pub struct VisibilityHardware {
    pipeline: Pipeline,
}
impl VisibilityHardware {
    /// Retain GreaterEqual depth writes so equal-depth fragments reach atomicMax.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        ensure!(
            gpu.info().buffer_int64_atomics,
            "hardware visibility needs unsigned 64-bit buffer min/max atomics"
        );
        let state = RenderState {
            depth_format: Some(Format::Depth32Float.vk().as_raw()),
            depth_test: true,
            depth_write: true,
            ..Default::default()
        };
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::VisibilityHardware, state)?,
        })
    }
    /// Draw a CPU-known visible count without vertex buffers. The config's packed
    /// viewport must match depth.size(); the caller retains compatible source tables.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        scene: &HardwareClusters<'_, '_>,
        depth: &RasterDepth,
        instances: u32,
        clear: bool,
    ) -> Result<()> {
        ensure!(
            instances > 0 && instances <= scene.count,
            "invalid visible hardware cluster count"
        );
        self.encode(
            graph,
            rec,
            scene,
            depth,
            clear,
            Command::DrawProcedural {
                vertices: 384,
                instances,
            },
        )
    }
    /// Consume the original finish kernel's 16-byte hardware draw record at byte16.
    /// Its GPU-produced instance count is at most scene.count; zero is a valid draw.
    pub fn record_indirect(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        scene: &HardwareClusters<'_, '_>,
        depth: &RasterDepth,
        arguments: &StorageBuffer,
        clear: bool,
    ) -> Result<()> {
        ensure!(arguments.len >= 32, "hardware draw arguments are too short");
        self.encode(
            graph,
            rec,
            scene,
            depth,
            clear,
            Command::DrawIndirect {
                arguments: arguments.buffer,
                offset: 16,
            },
        )
    }
    fn encode(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        scene: &HardwareClusters<'_, '_>,
        depth: &RasterDepth,
        clear: bool,
        command: Command,
    ) -> Result<()> {
        ensure!(
            scene.inputs.pass() == Pass::VisibilityHardware
                && scene.count > 0
                && depth.samples() == SampleCount::One,
            "invalid hardware visibility inputs"
        );
        let lists = scene
            .inputs
            .buffer(InputKey {
                group: 0,
                binding: 3,
            })
            .context("visibility list missing")?;
        let visibility = scene
            .inputs
            .buffer(InputKey {
                group: 0,
                binding: 8,
            })
            .context("visibility output missing")?;
        ensure!(
            u64::from(scene.count) * 2 <= lists.len / 4
                && u64::from(depth.size()[0]) * u64::from(depth.size()[1]) <= visibility.len / 8,
            "hardware visibility list/output too short"
        );
        if !clear {
            graph.require_initialized(Resource::Image(depth.image()))?;
        }
        let mut uses = scene.inputs.uses();
        uses.push(ResourceUse::Whole(
            Resource::Image(depth.image()),
            Access::Depth,
        ));
        if let Command::DrawIndirect { arguments, .. } = &command {
            uses.push(ResourceUse::Whole(
                Resource::Buffer(*arguments),
                Access::Indirect,
            ));
        }
        graph.node_uses(rec, &uses)?;
        rec.push(Command::BeginPass("ara::VisibilityHardware".into()));
        rec.push(Command::BeginRendering {
            colors: vec![],
            depth: Some(Attachment {
                image: depth.image(),
                clear: clear.then_some([0.; 4]),
            }),
            size: depth.size(),
        });
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(scene.inputs.roots()));
        rec.push(command);
        rec.push(Command::EndRendering);
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Retire after submitting all draws that reference this pipeline.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        gpu.retire_pipeline(self.pipeline)?;
        Ok(())
    }
}
