//! The original LC1 kernel, retaining light order and packed/truncated counts.
use super::{
    buffer::StorageBuffer, cache::ShaderCache, graph::Graph, image::Texture, shader::Pass,
};
use crate::light_cluster as lc;
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource, RootConstants,
    commands::{Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
};
/// Frozen light/camera records. Replace only at completion-safe submission boundaries.
pub struct ClusterInputs {
    /// Packed Renderlab Light records; at least one physical row for the empty case.
    pub lights: StorageBuffer,
    /// Exact existing LcFrame layout, including reserved shadow matrices/tiles.
    pub frame: StorageBuffer,
}
impl ClusterInputs {
    /// Reuse Renderlab's validated light packing and logarithmic cluster conventions.
    pub fn new(
        gpu: &mut GpuDevice,
        lighting: &crate::Lighting,
        camera: crate::Camera,
        size: [u32; 2],
    ) -> Result<Self> {
        ensure!(
            size.into_iter().all(|n| n > 0),
            "camera extent must be nonzero"
        );
        let packed = crate::lighting::PackedLighting::new(lighting, lc::MAX_CLUSTERED as u32)?;
        let mut records = packed.records;
        if records.is_empty() {
            records.resize(crate::lighting::LIGHT_BYTES, 0);
        }
        let lights = StorageBuffer::new(gpu, &records)?;
        let bytes = lc::header(&camera, size, packed.count, true, &[]);
        let frame = match StorageBuffer::new(gpu, &bytes) {
            Ok(frame) => frame,
            Err(e) => {
                lights.retire(gpu)?;
                return Err(e);
            }
        };
        Ok(Self { lights, frame })
    }
    /// Initial coherent writes must enter the graph before the first dispatch.
    pub fn import(&self, graph: &mut Graph) {
        graph.import(Resource::Buffer(self.lights.buffer), Access::HostWrite);
        graph.import(Resource::Buffer(self.frame.buffer), Access::HostWrite);
    }
    /// Retire immutable records after their last submitted use.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        self.lights.retire(gpu)?;
        self.frame.retire(gpu)
    }
}
/// One persistent LC1 culling pipeline. It creates no shadow/GI resources.
pub struct LightClusters {
    pipeline: Pipeline,
    threads: [u32; 3],
}
impl LightClusters {
    /// Load the original composed WGSL through the same native shader cache.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let threads = Pass::LightCluster.threads()?;
        ensure!(
            threads[1..] == [1, 1],
            "light clusters require one-dimensional workgroups"
        );
        Ok(Self {
            threads,
            pipeline: cache.pipeline(gpu, Pass::LightCluster, RenderState::default())?,
        })
    }
    /// Allocate the original packed list texture, including capacity for 128 indices.
    pub fn target(gpu: &mut GpuDevice) -> Result<Texture> {
        Texture::new(gpu, lc::LIST_SIZE, Format::R32Uint, true, false)
    }
    /// Record all 13,824 clusters, with one 64-thread workgroup per consecutive block.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &ClusterInputs,
        output: &Texture,
    ) -> Result<()> {
        ensure!(
            output.size == lc::LIST_SIZE && output.format == Format::R32Uint,
            "cluster list target mismatch"
        );
        let storage = output
            .storage
            .ok_or_else(|| anyhow::anyhow!("cluster list needs storage publication"))?;
        graph.node(
            rec,
            &[
                (Resource::Buffer(inputs.lights.buffer), Access::ShaderRead),
                (Resource::Buffer(inputs.frame.buffer), Access::ShaderRead),
                (Resource::Image(output.image), Access::Storage),
            ],
        )?;
        rec.push(Command::BeginPass("ara::LightClusters".into()));
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(RootConstants {
            words: [
                inputs.lights.slot,
                storage,
                inputs.frame.slot,
                0,
                0,
                0,
                0,
                0,
            ],
        }));
        rec.push(Command::Dispatch([
            lc::CLUSTERS.div_ceil(self.threads[0]),
            1,
            1,
        ]));
        rec.push(Command::EndPass);
        Ok(())
    }
}

/// The camera receivers' local-light visibility history (`lc::local_shadow_budget`,
/// shaders/local_light_history.wgsl): two RGBA32U images at the shading size. Frame `n`
/// writes image `n % 2` and reads the other, which frame `n - 1` wrote.
pub(crate) struct LocalHistory {
    images: [Texture; 2],
    /// What each image was last used as; None before it holds anything.
    last: [Option<Access>; 2],
}
impl LocalHistory {
    pub(crate) fn new(gpu: &mut GpuDevice, size: [u32; 2]) -> Result<Self> {
        Ok(Self {
            images: [
                Texture::new(gpu, size, Format::Rgba32Uint, true, false)?,
                Texture::new(gpu, size, Format::Rgba32Uint, true, false)?,
            ],
            last: [None; 2],
        })
    }
    pub(crate) fn size(&self) -> [u32; 2] {
        self.images[0].size
    }
    /// Declares both images to this frame's graph, zero-filling them on first use. Retire
    /// the returned staging buffers after the frame's list is submitted.
    pub(crate) fn begin(
        &mut self,
        gpu: &mut GpuDevice,
        graph: &mut Graph,
        rec: &mut Recorder,
    ) -> Result<Vec<ara_gpu::Buffer>> {
        let mut staging = Vec::new();
        for (image, last) in self.images.iter().zip(self.last) {
            match last {
                Some(access) => graph.import(Resource::Image(image.image), access),
                None => {
                    let texels = image.size[0] as usize * image.size[1] as usize;
                    staging.push(super::transfer::upload(
                        gpu,
                        graph,
                        rec,
                        image.image,
                        &vec![0; texels * 16],
                    )?)
                }
            }
        }
        Ok(staging)
    }
    /// Frame `frame`'s (read, written) images: bindings (3, 32) and (3, 33).
    pub(crate) fn images(&self, frame: u64) -> (&Texture, &Texture) {
        let write = (frame % 2) as usize;
        (&self.images[1 - write], &self.images[write])
    }
    /// Frame `frame` is recorded: its written image was last a storage target.
    pub(crate) fn advance(&mut self, frame: u64) {
        let write = (frame % 2) as usize;
        self.last[write] = Some(Access::Storage);
        self.last[1 - write] = Some(Access::ShaderRead);
    }
    pub(crate) fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for image in self.images {
            super::image::release(gpu, image.image)?;
        }
        Ok(())
    }
}
