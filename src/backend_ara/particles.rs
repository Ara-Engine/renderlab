//! Particle ordering leaf; simulation/lighting and billboard draw remain separate ports.
use super::{
    buffer::StorageBuffer,
    cache::ShaderCache,
    graph::Graph,
    shader::{ParticleResource as R, Pass},
};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, GpuDevice, Pipeline, Resource, RootConstants,
    commands::{Command, Recorder},
    cooked::RenderState,
};
/// Four published buffers with original TpFrame/TpEmitter/particle/key layouts.
pub struct ParticleSortBuffers {
    records: [StorageBuffer; 4],
    /// Live allocation capacity, not including power-of-two sorting padding.
    pub capacity: u32,
    /// Number of sorted keys, including dead/padded slots.
    pub sorted_len: u32,
}
impl ParticleSortBuffers {
    /// Each particle is twelve floats: position/age, velocity/lifetime, incident light.
    /// 2048 is the original shader's workgroup-array and 11-bit slot constraint.
    pub fn new(
        gpu: &mut GpuDevice,
        camera: crate::Camera,
        particles: &[[f32; 12]],
    ) -> Result<Self> {
        ensure!(
            !particles.is_empty() && particles.len() <= 2048,
            "particle sort supports1..2048 slots"
        );
        ensure!(
            particles.iter().flatten().all(|v| v.is_finite()),
            "invalid particle input"
        );
        let capacity = particles.len() as u32;
        let sorted_len = capacity.next_power_of_two();
        let mut frame = [0f32; 84];
        let eye = camera.view().inverse().w_axis.truncate();
        let forward = -camera.view().inverse().z_axis.truncate();
        frame[48..51].copy_from_slice(&eye.to_array());
        frame[52..55].copy_from_slice(&forward.to_array());
        let mut emitter = [0f32; 128];
        emitter[32] = f32::from_bits(capacity);
        emitter[37] = sorted_len as f32;
        let keys = vec![0xdeadbeefu32; sorted_len as usize];
        let bytes: [&[u8]; 4] = [
            bytemuck::cast_slice(&frame),
            bytemuck::cast_slice(&emitter),
            bytemuck::cast_slice(particles),
            bytemuck::cast_slice(&keys),
        ];
        let mut records = Vec::new();
        for bytes in bytes {
            match StorageBuffer::new(gpu, bytes) {
                Ok(b) => records.push(b),
                Err(e) => {
                    for b in records {
                        b.retire(gpu)?;
                    }
                    return Err(e);
                }
            }
        }
        Ok(Self {
            records: records.try_into().expect("four particle records"),
            capacity,
            sorted_len,
        })
    }
    /// Declare the coherent initialization before the first sort.
    pub fn import(&self, graph: &mut Graph) {
        for b in &self.records {
            graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
        }
    }
    /// Sorted full keys (depth top21bits, slot low11bits), for drawing or replay.
    pub fn keys(&self) -> &StorageBuffer {
        &self.records[R::Sorted as usize]
    }
    /// Declare retirement after submitting every list referencing these records.
    pub fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        for b in self.records {
            graph.forget(Resource::Buffer(b.buffer));
            b.retire(gpu)?;
        }
        Ok(())
    }
}
/// Persistent one-workgroup bitonic sort pipeline.
pub struct ParticleSort {
    pipeline: Pipeline,
}
impl ParticleSort {
    /// Compile the original kernel and retain its 2048-key threadgroup scratch.
    pub fn new(gpu: &mut GpuDevice, cache: &ShaderCache) -> Result<Self> {
        Ok(Self {
            pipeline: cache.pipeline(gpu, Pass::ParticleSort, RenderState::default())?,
        })
    }
    /// Sort far to near; dead/nonpositive-depth slots have zero depth keys.
    pub fn record(
        &self,
        graph: &mut Graph,
        rec: &mut Recorder,
        buffers: &ParticleSortBuffers,
    ) -> Result<()> {
        let uses = R::ALL.map(|r| {
            (
                Resource::Buffer(buffers.records[r as usize].buffer),
                if r.writes() {
                    Access::Storage
                } else {
                    Access::ShaderRead
                },
            )
        });
        graph.node(rec, &uses)?;
        let mut words = [0; 8];
        for r in R::ALL {
            words[r as usize] = buffers.records[r as usize].slot;
        }
        rec.push(Command::BeginPass("ara::ParticleSort".into()));
        rec.push(Command::Pipeline(self.pipeline));
        rec.push(Command::Constants(RootConstants { words }));
        rec.push(Command::Dispatch([1; 3]));
        rec.push(Command::EndPass);
        Ok(())
    }
}
