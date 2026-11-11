//! Temporal reconstruction (TR1), composed and planned as the wgpu renderer does.
//!
//! A traced frame is rendered through a camera jittered by the frame's sample of a
//! fixed sub-pixel sequence. After shading, the HDR target is copied aside and
//! `tr1_resolve` reprojects three history images, blends the new samples in and writes
//! the reconstructed image back into the HDR target. [`History`] is what a sequence
//! keeps: the history images on the GPU and Renderlab's own planner.
use super::{
    bindings::HeapBindings,
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    kernel::Kernel,
    shader::{InputKey, Pass, Program},
    transfer,
};
use crate::renderer::primary_ray_gpu::temporal_reconstruction::{PlanInputs, Planned, Planner};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Buffer, Device, GpuDevice, Resource, commands::Recorder, descriptors::Format,
};

/// `temporal_reconstruction::resolve_source` on the standard prelude: the non-RT
/// shading head, the object-motion source and the resolve itself. No ray queries.
pub(crate) fn resolve_source() -> Result<String> {
    let source = crate::renderer::primary_ray_gpu::temporal_reconstruction::resolve_source(
        include_str!("../../shaders/scene.wgsl"),
    )?;
    // MEASUREMENT ONLY: the one-pixel trace, when asked for.
    match super::resolve_probe::pixel() {
        Some(pixel) => super::resolve_probe::instrument(&source, pixel),
        None => Ok(source),
    }
}

/// The resolve kernel, reflected.
pub fn resolve_program() -> Result<Program> {
    Program::new(Pass::TemporalResolve, resolve_source()?)
}

/// What a frame sequence asks of the reconstruction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reconstruct {
    /// Enabled. Below native scale the frame renders at `RenderScale::internal_size`
    /// and the history and the resolved image are the scene's size.
    pub settings: crate::Reconstruction,
    /// Copy this frame's history colour out: per pixel the exposed mean in rgb and
    /// the accumulated sample weight in alpha, as RGBA32F.
    pub read_history: bool,
}

/// The reconstruction across frames: `ReconstructionState`'s three history image
/// pairs, ping-ponged by the planner's parity, the planner and the sample index.
pub struct History {
    settings: crate::Reconstruction,
    size: [u32; 2],
    /// Exposed mean and weight (RGBA32F), anchors (RG32U), silhouette slots (RGBA32U).
    colour: [Texture; 2],
    anchor: [Texture; 2],
    layer: [Texture; 2],
    /// What each side's colour, anchor and layer images were last used as; None
    /// before they hold anything.
    last: [[Option<Access>; 3]; 2],
    plan: Planner,
    /// `Renderer::reconstruction_index`: this frame's place in the jitter sequence.
    index: u64,
    program: Program,
    kernel: Kernel,
}
impl History {
    pub fn new(
        gpu: &mut GpuDevice,
        cache: &ShaderCache,
        settings: crate::Reconstruction,
        size: [u32; 2],
    ) -> Result<Self> {
        settings
            .validate()
            .map_err(|e| anyhow::anyhow!("reconstruction settings: {e}"))?;
        ensure!(settings.enabled, "reconstruction history needs it enabled");
        let program = resolve_program()?;
        let kernel = Kernel::from_program(gpu, cache, &program)?;
        let mut pair = |format| -> Result<[Texture; 2]> {
            Ok([
                Texture::new(gpu, size, format, true, false)?,
                Texture::new(gpu, size, format, true, false)?,
            ])
        };
        Ok(Self {
            settings,
            size,
            colour: pair(Format::Rgba32Float)?,
            anchor: pair(Format::Rg32Uint)?,
            layer: pair(Format::Rgba32Uint)?,
            last: [[None; 3]; 2],
            plan: Planner::new(),
            index: 0,
            program,
            kernel,
        })
    }
    pub(crate) fn matches(&self, settings: crate::Reconstruction, size: [u32; 2]) -> bool {
        self.settings == settings && self.size == size
    }
    /// What the planner decided for the last planned frame: whether it restarted every
    /// pixel, and the Interactive shadow ray budget (`Renderer::reconstruction_shadow_rays`).
    pub fn planned(&self) -> (bool, u32) {
        (self.plan.reset, self.plan.shadow_rays)
    }
    /// This frame's index into the jitter sequence.
    pub(crate) fn index(&self) -> u64 {
        self.index
    }
    /// `Renderer::cut_reconstruction`: the next frame restarts every pixel. The sample
    /// index runs on.
    pub fn cut(&mut self) {
        self.plan.cut = true;
    }
    /// Declares the history images' states to this frame's graph, zero-filling them on
    /// first use (wgpu zero-initialises its textures). Retire the returned staging
    /// buffers after the frame's list is submitted.
    pub(crate) fn begin(
        &mut self,
        gpu: &mut GpuDevice,
        graph: &mut Graph,
        rec: &mut Recorder,
    ) -> Result<Vec<Buffer>> {
        let mut staging = Vec::new();
        let texels = self.size[0] as usize * self.size[1] as usize;
        for side in 0..2 {
            for (kind, (image, bytes)) in [
                (&self.colour[side], 16),
                (&self.anchor[side], 8),
                (&self.layer[side], 16),
            ]
            .into_iter()
            .enumerate()
            {
                match self.last[side][kind] {
                    Some(access) => graph.import(Resource::Image(image.image), access),
                    None => staging.push(transfer::upload(
                        gpu,
                        graph,
                        rec,
                        image.image,
                        &vec![0; texels * bytes],
                    )?),
                }
            }
        }
        Ok(staging)
    }
    /// `Planner::prepare`: the frame's records, with the history advanced.
    pub(crate) fn plan(&mut self, inputs: PlanInputs<'_>) -> Planned {
        self.plan.prepare(inputs)
    }
    /// Whether the frame just planned restarted every pixel.
    pub(crate) fn reset(&self) -> bool {
        self.plan.reset
    }
    /// The history image a resolve input names, after `plan`: the side the parity
    /// selects is written (44, 45, 48), the other read (42, 43, 47).
    pub(crate) fn input(&self, key: InputKey) -> Option<&Texture> {
        let (write, read) = (self.plan.parity, 1 - self.plan.parity);
        Some(match (key.group, key.binding) {
            (0, 42) => &self.colour[read],
            (0, 43) => &self.anchor[read],
            (0, 47) => &self.layer[read],
            (0, 44) => &self.colour[write],
            (0, 45) => &self.anchor[write],
            (0, 48) => &self.layer[write],
            _ => return None,
        })
    }
    pub(crate) fn program(&self) -> &Program {
        &self.program
    }
    /// The colour history this frame writes.
    pub(crate) fn written(&self) -> &Texture {
        &self.colour[self.plan.parity]
    }
    /// `encode_reconstruction`'s dispatch: 8x8 groups over the output.
    pub(crate) fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        packet: &HeapBindings<'_>,
    ) -> Result<()> {
        self.kernel
            .record(graph, rec, packet, [self.size[0], self.size[1], 1])
    }
    /// The frame is recorded: remember what each side was last used as (`read_back`:
    /// the written colour was copied out) and move to the next sample.
    pub(crate) fn advance(&mut self, read_back: bool) {
        let write = self.plan.parity;
        let colour = if read_back {
            Access::CopySource
        } else {
            Access::Storage
        };
        self.last[write] = [Some(colour), Some(Access::Storage), Some(Access::Storage)];
        self.last[1 - write] = [Some(Access::ShaderRead); 3];
        self.index += 1;
    }
    /// Retire after every list that names the history has been submitted.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for image in self.colour.into_iter().chain(self.anchor).chain(self.layer) {
            gpu.retire(Resource::Image(image.image))?;
        }
        Ok(())
    }
}
