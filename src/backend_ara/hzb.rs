//! Original reversed-Z minimum pyramid over one persistent image allocation.
use super::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    cache::ShaderCache,
    graph::Graph,
    image::Texture,
    kernel::Kernel,
    shader::{InputKey, Pass},
};
use anyhow::{Context, Result, ensure};
use ara_gpu::{Access, GpuDevice, Image, Resource, commands::Recorder, descriptors::Format};
/// Persistent source-size record, mip views and the two original reduction kernels.
pub struct Hzb {
    size: [u32; 2],
    params: StorageBuffer,
    parent: Texture,
    views: Vec<Texture>,
    base: Kernel,
    reduce: Kernel,
}
/// Prepared source/output indices. Retire before its borrowed pyramid and source.
pub struct HzbInputs<'a> {
    image: Image,
    base: HeapBindings<'a>,
    reduce: Vec<HeapBindings<'a>>,
}
fn key(binding: u32) -> InputKey {
    InputKey { group: 0, binding }
}
impl Hzb {
    /// Preserve Renderlab's power-of-two half-resolution base and full mip chain.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache, size: [u32; 2]) -> Result<Self> {
        ensure!(size.iter().all(|n| *n > 0), "empty HZB source");
        let extent = [
            size[0]
                .div_ceil(2)
                .checked_next_power_of_two()
                .context("HZB width overflow")?,
            size[1]
                .div_ceil(2)
                .checked_next_power_of_two()
                .context("HZB height overflow")?,
        ];
        let levels = 32 - extent[0].max(extent[1]).leading_zeros();
        let base = Kernel::new(gpu, cache, Pass::HzbBase)?;
        let reduce = match Kernel::new(gpu, cache, Pass::HzbReduce) {
            Ok(p) => p,
            Err(e) => {
                return Err(e);
            }
        };
        let params = match StorageBuffer::new(gpu, bytemuck::cast_slice(&[size[0], size[1], 0, 0]))
        {
            Ok(p) => p,
            Err(e) => {
                return Err(e);
            }
        };
        let (parent, views) = match Texture::mip_chain(gpu, extent, levels, Format::R32Float) {
            Ok(v) => v,
            Err(e) => {
                params.retire(gpu)?;
                return Err(e);
            }
        };
        Ok(Self {
            size,
            params,
            parent,
            views,
            base,
            reduce,
        })
    }
    /// Parent sampled view and identity, for later conservative visibility tests.
    pub fn texture(&self) -> &Texture {
        &self.parent
    }
    /// Individual narrow views in level order, for explicit consumers and replay.
    pub fn levels(&self) -> &[Texture] {
        &self.views
    }
    /// Publish immutable indices once. Visibility records are two u32 words per
    /// pixel: the shader reads reversed-Z float bits from the second word.
    pub fn inputs<'a>(
        &'a self,
        gpu: &mut GpuDevice,
        visibility: &'a StorageBuffer,
    ) -> Result<HzbInputs<'a>> {
        let bytes = u64::from(self.size[0])
            .checked_mul(u64::from(self.size[1]))
            .and_then(|n| n.checked_mul(8))
            .context("HZB source footprint overflow")?;
        ensure!(visibility.len >= bytes, "HZB visibility buffer too short");
        let base = HeapBindings::new(
            gpu,
            Pass::HzbBase,
            &[
                (key(0), Input::Buffer(visibility)),
                (key(1), Input::Image(&self.views[0])),
                (key(3), Input::Buffer(&self.params)),
            ],
        )?;
        let mut reduce = Vec::new();
        for levels in self.views.windows(2) {
            match HeapBindings::new(
                gpu,
                Pass::HzbReduce,
                &[
                    (key(1), Input::Image(&levels[1])),
                    (key(2), Input::Image(&levels[0])),
                ],
            ) {
                Ok(packet) => reduce.push(packet),
                Err(e) => {
                    let mut unused = Graph::default();
                    base.retire(gpu, &mut unused)?;
                    for p in reduce {
                        p.retire(gpu, &mut unused)?;
                    }
                    return Err(e);
                }
            }
        }
        Ok(HzbInputs {
            image: self.parent.image,
            base,
            reduce,
        })
    }
    /// Import immutable record initialization once; caller imports/updates visibility.
    pub fn import(&self, graph: &mut Graph) {
        graph.import(Resource::Buffer(self.params.buffer), Access::HostWrite);
    }
    /// Rebuild all mips with precise read/write ranges and no intermediate CPU waits.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        inputs: &HzbInputs<'_>,
    ) -> Result<()> {
        ensure!(
            inputs.image == self.parent.image,
            "HZB packet belongs to another pyramid"
        );
        self.base.record(
            graph,
            rec,
            &inputs.base,
            [self.views[0].size[0], self.views[0].size[1], 1],
        )?;
        for (packet, view) in inputs.reduce.iter().zip(&self.views[1..]) {
            self.reduce
                .record(graph, rec, packet, [view.size[0], view.size[1], 1])?;
        }
        Ok(())
    }
    /// Retire the shared allocation once, after the prepared packets are retired.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        graph.forget(Resource::Image(self.parent.image));
        graph.forget(Resource::Buffer(self.params.buffer));
        super::image::release(gpu, self.parent.image)?;
        self.params.retire(gpu)
    }
}
impl HzbInputs<'_> {
    /// Import persistent index records once, without replacing producer states.
    pub fn import(&self, graph: &mut Graph) {
        self.base.import(graph);
        for p in &self.reduce {
            p.import(graph);
        }
    }
    /// Release resource borrows after every referencing list has been submitted.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        self.base.retire(gpu, graph)?;
        for p in self.reduce {
            p.retire(gpu, graph)?;
        }
        Ok(())
    }
}
