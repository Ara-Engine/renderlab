//! Raster post auxiliary data, matching the original single/sample-zero variants.
use super::{
    buffer::StorageBuffer, cache::ShaderCache, graph::Graph, image::Texture, shader::Pass,
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Device, GpuDevice, Image, Pipeline, Resource, RootConstants,
    commands::{Command, Recorder},
    cooked::RenderState,
    descriptors::{Format, ImageDesc},
    texture::{ImageShape, SampleCount, TextureDesc},
};

/// A sampled raster depth attachment, with its shader sample type retained.
pub struct RasterDepth {
    image: Image,
    size: [u32; 2],
    samples: SampleCount,
    slot: u32,
}
impl RasterDepth {
    /// Allocate backed depth storage; all samples remain readable after rendering.
    pub fn new(gpu: &mut GpuDevice, size: [u32; 2], samples: SampleCount) -> Result<Self> {
        let image = gpu.create_texture(&TextureDesc {
            image: ImageDesc {
                size,
                format: Format::Depth32Float,
                sampled: true,
                storage: false,
                attachment: true,
                copy_source: samples == SampleCount::One,
                copy_destination: false,
            },
            shape: ImageShape {
                samples,
                ..Default::default()
            },
        })?;
        match gpu.sampled_image(image, true) {
            Ok(slot) => Ok(Self {
                image,
                size,
                samples,
                slot: slot.index,
            }),
            Err(e) => {
                gpu.retire(Resource::Image(image))?;
                Err(e.into())
            }
        }
    }
    /// Attachment identity for the renderer's main raster pass.
    pub fn image(&self) -> Image {
        self.image
    }
    /// Attachment width and height.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    /// Backed attachment sample count.
    pub fn samples(&self) -> SampleCount {
        self.samples
    }
    /// Retire after all submitted raster/auxiliary uses and remove its scheduled state.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        graph.forget(Resource::Image(self.image));
        Ok(gpu.retire(Resource::Image(self.image))?)
    }
}
/// Exact 272-byte current/previous-camera record used by the original kernel.
pub struct PostCameraFrame {
    /// Storage record to import as HostWrite once, then ShaderRead.
    pub record: StorageBuffer,
}
impl PostCameraFrame {
    /// Previous is the previous admitted frame's unjittered view-projection matrix.
    /// None suppresses motion on camera cuts. Radius is in output pixels.
    pub fn new(
        gpu: &mut GpuDevice,
        camera: crate::Camera,
        previous: Option<crate::Mat4>,
        size: [u32; 2],
        shutter_angle: f32,
        max_radius: f32,
    ) -> Result<Self> {
        ensure!(
            size.into_iter().all(|n| n > 0)
                && shutter_angle.is_finite()
                && (0.0..=360.0).contains(&shutter_angle)
                && max_radius.is_finite()
                && (0.0..=64.0).contains(&max_radius),
            "invalid post camera extent/shutter/radius"
        );
        let current = camera.unjittered().matrix(size[0], size[1]);
        let mut words = Vec::with_capacity(68);
        for matrix in [
            previous.unwrap_or(current),
            current,
            camera.matrix(size[0], size[1]).inverse(),
            camera.view(),
        ] {
            ensure!(matrix.is_finite(), "nonfinite post camera matrix");
            words.extend(matrix.to_cols_array());
        }
        words.extend([
            shutter_angle / 720.,
            max_radius,
            if previous.is_some() { 1. } else { 0. },
            0.,
        ]);
        Ok(Self {
            record: StorageBuffer::new(gpu, bytemuck::cast_slice(&words))?,
        })
    }
}
/// Persistent depth-to-velocity/depth pipelines for 1x and 4x raster targets.
pub struct RasterPostSurface {
    pipelines: [Pipeline; 2],
    threads: [[u32; 3]; 2],
}
impl RasterPostSurface {
    /// Compile the same WGSL with the exact existing MSAA depth-type substitution.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let stages = [Pass::RasterSurface, Pass::RasterSurfaceFour];
        let threads = stages
            .map(Pass::threads)
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .expect("two raster surface variants");
        let first = cache.pipeline(gpu, stages[0], RenderState::default())?;
        let second = match cache.pipeline(gpu, stages[1], RenderState::default()) {
            Ok(p) => p,
            Err(e) => {
                gpu.retire_pipeline(first)?;
                return Err(e);
            }
        };
        Ok(Self {
            pipelines: [first, second],
            threads,
        })
    }
    /// Produce output-space shutter velocity and linear view depth for post filters.
    /// Object-motion from primary visibility uses its separate later surface pass.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        depth: &RasterDepth,
        frame: &PostCameraFrame,
        output: &Texture,
    ) -> Result<()> {
        ensure!(
            depth.size == output.size && output.format == Format::Rgba32Float,
            "raster auxiliary shape/format mismatch"
        );
        let i = usize::from(depth.samples == SampleCount::Four);
        let storage = output
            .storage
            .ok_or_else(|| anyhow::anyhow!("raster surface output needs storage publication"))?;
        graph.node(
            rec,
            &[
                (Resource::Image(depth.image), Access::ShaderRead),
                (Resource::Buffer(frame.record.buffer), Access::ShaderRead),
                (Resource::Image(output.image), Access::Storage),
            ],
        )?;
        rec.push(Command::BeginPass("ara::RasterPostSurface".into()));
        rec.push(Command::Pipeline(self.pipelines[i]));
        rec.push(Command::Constants(RootConstants {
            words: [depth.slot, frame.record.slot, storage, 0, 0, 0, 0, 0],
        }));
        rec.push(Command::Dispatch([
            output.size[0].div_ceil(self.threads[i][0]),
            output.size[1].div_ceil(self.threads[i][1]),
            1,
        ]));
        rec.push(Command::EndPass);
        Ok(())
    }
    /// Retire both variants after their last submitted use.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for p in self.pipelines {
            gpu.retire_pipeline(p)?;
        }
        Ok(())
    }
}
