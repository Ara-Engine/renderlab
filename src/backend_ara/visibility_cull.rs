//! Original two-phase cluster classification and working-state lifecycle.
use super::{
    bindings::HeapBindings, buffer::StorageBuffer, cache::ShaderCache, graph::Graph,
    kernel::Kernel, shader::Pass,
};
use anyhow::{Result, ensure};
use ara_gpu::{GpuDevice, commands::Recorder};
/// Persistent cull, finish and reset kernels; pass-specific packets own all indices.
pub struct VisibilityCull {
    cull: Kernel,
    finish: Kernel,
    reset: Kernel,
}
impl VisibilityCull {
    /// Compile the original stage compositions without a geometry/shading rewrite.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        let cull = Kernel::new(gpu, cache, Pass::VisibilityCull)?;
        let finish = match Kernel::new(gpu, cache, Pass::VisibilityFinish) {
            Ok(p) => p,
            Err(e) => {
                cull.retire(gpu)?;
                return Err(e);
            }
        };
        let reset = match Kernel::new(gpu, cache, Pass::VisibilityReset) {
            Ok(p) => p,
            Err(e) => {
                cull.retire(gpu)?;
                finish.retire(gpu)?;
                return Err(e);
            }
        };
        Ok(Self {
            cull,
            finish,
            reset,
        })
    }
    /// Classify the original cluster set. Rows retain the source 65535-group stride.
    /// Working counters must be initialized/reset, as on the existing wgpu path.
    pub fn first(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        clusters: u32,
    ) -> Result<()> {
        ensure!(
            inputs.pass() == Pass::VisibilityCull,
            "wrong cluster-cull packet"
        );
        if clusters == 0 {
            return Ok(());
        }
        let groups = clusters.div_ceil(64);
        self.cull.record(
            graph,
            rec,
            inputs,
            [groups.min(65535) * 64, groups.div_ceil(65535), 1],
        )
    }
    /// Retest only pass1's rejects via the GPU-produced next_x/y/z at byte48.
    /// The second packet binds separate output arguments, preserving indirect safety.
    pub fn second(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
        first_arguments: &StorageBuffer,
    ) -> Result<()> {
        self.cull
            .record_indirect(graph, rec, inputs, first_arguments, 48)
    }
    /// Convert this pass's counters into raster, next-cull and per-mip dispatches.
    pub fn finish(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
    ) -> Result<()> {
        self.finish.record(graph, rec, inputs, [1, 1, 1])
    }
    /// Copy 16 working words to diagnostics and zero them; pass0 also clears the
    /// shared reject header. Invoke after consuming the pass's indirect outputs.
    pub fn reset(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
    ) -> Result<()> {
        self.reset.record(graph, rec, inputs, [128, 1, 1])
    }
    /// Retire after all referencing lists have been submitted.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        self.cull.retire(gpu)?;
        self.finish.retire(gpu)?;
        self.reset.retire(gpu)
    }
}
