//! Original geometric shadow queries; no raster visibility map or history is involved.
use super::{
    bindings::HeapBindings,
    cache::ShaderCache,
    graph::Graph,
    kernel::Kernel,
    shader::{InputKey, Pass, Program},
};
use anyhow::{Context, Result, ensure};
use ara_gpu::{GpuDevice, commands::Recorder, descriptors::Format};
/// Source estimator specialization, retaining the same 0..256 output count contract.
#[derive(Clone, Copy)]
pub enum Mode {
    /// 256 fresh directions, eight cooperating lanes per receiver in a 2x2 tile.
    Exact,
    /// Eight fresh directions, one lane per receiver in an 8x4 tile.
    Interactive,
}
impl Mode {
    fn tile(self) -> [u32; 2] {
        match self {
            Self::Exact => [2, 2],
            Self::Interactive => [8, 4],
        }
    }
    /// Compose the production source and specialize only its existing direction override.
    pub fn program(self) -> Result<Program> {
        Program::new(Pass::FreshVisibility, source(self)?)
    }
}
pub(crate) fn source(mode: Mode) -> Result<String> {
    let source =
        crate::renderer::fresh_shadow_gpu::shader_source(include_str!("../../shaders/scene.wgsl"))?;
    if matches!(mode, Mode::Interactive) {
        let marker = "override FRESH_DIRECTIONS:u32=256u;";
        ensure!(
            source.matches(marker).count() == 1,
            "fresh direction override changed"
        );
        Ok(source.replace(marker, "override FRESH_DIRECTIONS:u32=8u;"))
    } else {
        Ok(source)
    }
}
/// Persistent fresh_compute pipeline and its source-specific receiver tile shape.
pub struct Visibility {
    kernel: Kernel,
    tile: [u32; 2],
}
impl Visibility {
    /// Compile the original hardware-query estimator for one direction budget.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, mode: Mode) -> Result<Self> {
        let program = mode.program()?;
        ensure!(
            program.threads()? == [32, 1, 1],
            "fresh workgroup shape changed"
        );
        Ok(Self {
            kernel: Kernel::from_program(gpu, cache, &program)?,
            tile: mode.tile(),
        })
    }
    /// Read matching world/normal receiver images and write every output pixel,
    /// including uncovered pixels. The packet retains its published TLAS and tables.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HeapBindings<'_>,
    ) -> Result<()> {
        let image = |binding| {
            inputs
                .image(InputKey { group: 0, binding })
                .context("fresh visibility image missing")
        };
        let world = image(3)?;
        let normal = image(4)?;
        let output = image(5)?;
        ensure!(
            world.format == Format::Rgba32Float
                && normal.format == Format::Rgba32Float
                && output.format == Format::R32Uint
                && world.size == normal.size
                && world.size == output.size,
            "fresh visibility receiver/output mismatch"
        );
        let [width, height] = world.size;
        let x = width
            .div_ceil(self.tile[0])
            .checked_mul(32)
            .context("fresh visibility dispatch overflow")?;
        self.kernel
            .record(graph, rec, inputs, [x, height.div_ceil(self.tile[1]), 1])
    }
    /// Retire after the last referencing submission.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        self.kernel.retire(gpu)
    }
}
/// The estimator as a traced frame composes it (`primary_ray_gpu::shader_sources`):
/// the same `fresh_compute`, with coverage gated on the primary tiles and hit image
/// its camera trace wrote. Reflected and compiled natively on Metal; not yet executed
/// against a reference.
pub fn traced_program() -> Result<Program> {
    let [_, soft] =
        crate::renderer::primary_ray_gpu::shader_sources(include_str!("../../shaders/scene.wgsl"))?;
    Program::new(Pass::FreshVisibility, soft)
}
