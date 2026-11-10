//! Caller-authored compute WGSL with one immutable reflected packet contract.
use super::{Pass, ShaderInput};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
/// Prepared WGSL for a selected compute entry family. Prepare outside recording;
/// pipelines and packets retain their own contract identity after this is dropped.
pub struct Program {
    pass: Pass,
    source: String,
    inputs: Vec<ShaderInput>,
    threads: [u32; 3],
    contract: [u8; 32],
}
impl Program {
    /// Validate caller WGSL and reflect only the chosen entry. The family selects
    /// its entry name and wrapper convention, while the program owns its code.
    pub fn new(pass: Pass, source: impl Into<String>) -> Result<Self> {
        ensure!(
            pass.indirect() && !pass.raster(),
            "program needs a compute packet family"
        );
        let source = source.into();
        let inputs = super::heap_interface::from_source(pass, &source)?;
        let threads = super::threads_source(pass, &source)?;
        let contract = contract(pass, &inputs);
        Ok(Self {
            pass,
            source,
            inputs,
            threads,
            contract,
        })
    }
    /// Ordered source keys and resource requirements for this program's packet.
    pub fn inputs(&self) -> &[ShaderInput] {
        &self.inputs
    }
    /// Selected compute family.
    pub fn pass(&self) -> Pass {
        self.pass
    }
    /// Resolved workgroup size, including source override defaults.
    pub fn threads(&self) -> [u32; 3] {
        self.threads
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
}

pub(crate) fn contract(pass: Pass, inputs: &[ShaderInput]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(format!("{pass:?}"));
    for input in inputs {
        hash.update(format!("{:?}:{:?};", input.key, input.kind));
    }
    hash.finalize().into()
}
