//! Caller-authored WGSL with one immutable reflected packet contract.
use super::{Pass, ShaderInput};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
/// Prepared WGSL for a selected compute or raster entry family. Prepare outside recording;
/// pipelines and packets retain their own contract identity after this is dropped.
pub struct Program {
    pass: Pass,
    source: String,
    inputs: Vec<ShaderInput>,
    threads: Option<[u32; 3]>,
    contract: [u8; 32],
    /// The source's digest, taken once: its key in the reflection tables and in the
    /// pipeline cache.
    digest: [u8; 32],
}
impl Program {
    /// Validate caller WGSL and reflect only the chosen entry or raster pair. The family selects
    /// its entry name and wrapper convention, while the program owns its code.
    pub fn new(pass: Pass, source: impl Into<String>) -> Result<Self> {
        ensure!(pass.indirect(), "program needs a reflected packet family");
        let source = source.into();
        let digest = super::memo::digest(&source);
        let inputs = super::heap_interface::from_digest(pass, &source, digest)?;
        let threads = if pass.raster() {
            None
        } else {
            Some(super::threads_digest(pass, &source, digest)?)
        };
        let contract = contract(pass, &inputs);
        Ok(Self {
            pass,
            source,
            inputs,
            threads,
            contract,
            digest,
        })
    }
    /// Ordered source keys and resource requirements for this program's packet.
    pub fn inputs(&self) -> &[ShaderInput] {
        &self.inputs
    }
    /// Selected compute or raster family.
    pub fn pass(&self) -> Pass {
        self.pass
    }
    /// Resolved compute workgroup size; raster programs return an error.
    pub fn threads(&self) -> Result<[u32; 3]> {
        self.threads
            .context("raster programs have no compute workgroup size")
    }
    /// Emit native Metal source through the shared heap adapter.
    pub fn msl(&self) -> Result<super::MetalProgram> {
        super::metal::from_source(self.pass, &self.source)
    }
    /// Emit SPIR-V through the shared heap adapter.
    pub fn spirv(&self) -> Result<Vec<super::SpirvStage>> {
        super::spirv::from_source(self.pass, &self.source)
    }
    pub(crate) fn source(&self) -> &str {
        &self.source
    }
    pub(crate) fn contract(&self) -> [u8; 32] {
        self.contract
    }
    pub(crate) fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

pub(crate) fn contract(pass: Pass, inputs: &[ShaderInput]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(format!("{pass:?}"));
    for input in inputs {
        hash.update(format!("{:?}:{:?};", input.key, input.kind));
    }
    hash.finalize().into()
}
