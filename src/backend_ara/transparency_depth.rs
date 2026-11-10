//! Original opaque-depth conversion leaf for later transparency and particles.
use super::{
    buffer::StorageBuffer, cache::ShaderCache, graph::Graph, image::Texture,
    post_surface::RasterDepth, shader::Pass,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource, RootConstants,
    commands::{Command, Recorder},
    cooked::RenderState,
    descriptors::Format,
    texture::{SampleCount, TextureDimension},
};
/// Original336-byte TpFrame record, populated for the depth conversion's reads.
pub struct RasterDepthFrame {
    /// Storage record to import as HostWrite before its first dispatch.
    pub record: StorageBuffer,
    size: [u32; 2],
}
impl RasterDepthFrame {
    /// Match the exact camera/projection that produced the raster attachment.
    pub fn new(gpu: &mut GpuDevice, camera: crate::Camera, size: [u32; 2]) -> Result<Self> {
        ensure!(size.into_iter().all(|n| n > 0), "empty raster depth frame");
        let matrix = camera.matrix(size[0], size[1]);
        let inverse = matrix.inverse();
        ensure!(inverse.is_finite(), "invalid raster projection inverse");
        let view = camera.view().inverse();
        let eye = view.w_axis.truncate();
        let forward = -view.z_axis.truncate();
        let mut words = [0f32; 84];
        words[..16].copy_from_slice(&matrix.to_cols_array());
        words[16..32].copy_from_slice(&inverse.to_cols_array());
        words[48..51].copy_from_slice(&eye.to_array());
        words[52..55].copy_from_slice(&forward.to_array());
        words[64..68].copy_from_slice(&[
            size[0] as f32,
            size[1] as f32,
            1. / size[0] as f32,
            1. / size[1] as f32,
        ]);
        Ok(Self {
            record: StorageBuffer::new(gpu, bytemuck::cast_slice(&words))?,
            size,
        })
    }
}
/// Persistent single-sample depth-to-distance pipeline.
pub struct RasterDepthConversion {
    pipeline: Pipeline,
    threads: [u32; 3],
}
impl RasterDepthConversion {
    /// Compile the unchanged TP_FRAME + TP_DEPTH_RASTER source sections.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        Ok(Self {
            threads: Pass::TransparencyDepth.threads()?,
            pipeline: cache.pipeline(gpu, Pass::TransparencyDepth, RenderState::default())?,
        })
    }
    /// Convert positive depth to view distance; original cleared-depth misses stay1e30.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        frame: &RasterDepthFrame,
        depth: &RasterDepth,
        output: &Texture,
    ) -> Result<()> {
        ensure!(
            depth.samples() == SampleCount::One
                && depth.size() == frame.size
                && output.size == frame.size
                && output.format == Format::R32Float
                && output.shape.dimension == TextureDimension::D2
                && output.shape.layers == 1
                && output.shape.mip_levels == 1
                && output.shape.samples == SampleCount::One,
            "raster depth conversion target mismatch"
        );
        let storage = output
            .storage
            .ok_or_else(|| anyhow::anyhow!("view-distance output needs storage access"))?;
        graph.node(
            rec,
            &[
                (Resource::Buffer(frame.record.buffer), Access::ShaderRead),
                (Resource::Image(depth.image()), Access::ShaderRead),
                (Resource::Image(output.image), Access::Storage),
            ],
        )?;
        rec.push(Command::BeginPass("ara::TransparencyDepth".into()));
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(RootConstants {
            words: [depth.sampled(), frame.record.slot, storage, 0, 0, 0, 0, 0],
        }));
        rec.push(Command::Dispatch([
            frame.size[0].div_ceil(self.threads[0]),
            frame.size[1].div_ceil(self.threads[1]),
            1,
        ]));
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Retire after submitting the last conversion.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        gpu.retire_pipeline(self.pipeline)?;
        Ok(())
    }
}
