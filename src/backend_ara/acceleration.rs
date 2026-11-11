//! Published TLAS owner for ray-query heap packets; BLAS creation stays in ara-gpu.
use anyhow::Result;
use ara_gpu::{
    AccelerationStructure, BindlessHeap, Device, GpuDevice, Resource,
    acceleration::{AccelerationDesc, AccelerationInstance},
};
/// One immutable TLAS publication. Retire after submitting every referencing list.
pub struct Scene {
    /// Generational allocation identity, also used for build/read graph edges.
    pub acceleration: AccelerationStructure,
    pub(crate) slot: u32,
}
impl Scene {
    /// Allocate and publish a TLAS over existing BLAS handles. Record its build
    /// after building its geometry, then transition it to AsRead before tracing.
    pub fn new(gpu: &mut GpuDevice, instances: Vec<AccelerationInstance>) -> Result<Self> {
        let acceleration =
            gpu.create_acceleration_structure(&AccelerationDesc::Instances(instances))?;
        match gpu.heap().insert_acceleration_structure(acceleration) {
            Ok(index) => Ok(Self {
                acceleration,
                slot: index.index,
            }),
            Err(error) => {
                gpu.retire(Resource::AccelerationStructure(acceleration))?;
                Err(error.into())
            }
        }
    }
    /// Retire this TLAS and its publication after submitting referencing lists.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        Ok(gpu.retire(Resource::AccelerationStructure(self.acceleration))?)
    }
}
