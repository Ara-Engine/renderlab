//! Content-addressed native artifacts; production pipeline loading stays in ara-gpu.
use super::shader::{self, Pass};
#[cfg(target_os = "macos")]
use anyhow::Context;
use anyhow::{Result, ensure};
use ara_gpu::{
    GpuDevice, Pipeline,
    cooked::{self, CookedPipeline, CookedStage, RenderState, Stage},
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Owned native shader cache. Construct outside frame recording.
///
/// A pipeline belongs to the cache that made it. The cache makes each one once and
/// hands the same handle to every caller that asks for the same WGSL, entry and
/// state, frame after frame; [`retire`](Self::retire) releases them all. The passes
/// that draw with a pipeline do not release it.
pub struct ShaderCache {
    root: PathBuf,
    compiler: String,
    /// The digest of everything a pipeline's key holds besides its own WGSL, entry
    /// and state: the adapter's sources, the compiler and the binding layout.
    salt: [u8; 32],
    /// The pipelines made so far, by key.
    pipelines: std::cell::RefCell<std::collections::HashMap<String, Pipeline>>,
    /// Each pass's own source's digest, taken once.
    digests: std::cell::RefCell<std::collections::HashMap<Pass, [u8; 32]>>,
}
impl ShaderCache {
    /// Create an explicit cache directory; no files are written to the user profile.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        #[cfg(target_os = "macos")]
        let compiler = {
            let output = std::process::Command::new("xcrun").args(["metal", "--version"]).output()
                .context("Apple Metal toolchain unavailable; run xcodebuild -downloadComponent MetalToolchain")?;
            ensure!(
                output.status.success(),
                "Apple Metal toolchain unavailable: {}. Run xcodebuild -downloadComponent MetalToolchain",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        #[cfg(not(target_os = "macos"))]
        let compiler = String::from("naga30 SPIR-V1.5");
        let adapter = format!(
            "{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}",
            include_str!("shader/metal.rs"),
            include_str!("shader/spirv.rs"),
            include_str!("shader/mod.rs"),
            include_str!("shader/skin_interface.rs"),
            include_str!("shader/particle_interface.rs"),
            include_str!("shader/mesh_interface.rs"),
            include_str!("shader/heap_interface.rs"),
            include_str!("shader/packet_spirv.rs"),
            include_str!("shader/spv_layout.rs"),
            include_str!("shader/spv_nonuniform.rs"),
            include_str!("shader/spv_precise.rs"),
            include_str!("shader/spv_acceleration.rs"),
            include_str!("shader/array_access.rs"),
            include_str!("shader/buffer_arrays.rs"),
            include_str!("shader/handle_arrays.rs"),
            include_str!("shader/program.rs"),
            include_str!("shader/local_shadows.rs"),
            include_str!("shader/ray_source.rs"),
            include_str!("cache.rs"),
            include_str!("../../Cargo.lock")
        );
        let salt = Sha256::digest(format!(
            "renderlab-ara-v2:{adapter}:{compiler}:{}:{}",
            include_str!("shader/abi.metal"),
            cooked::binding_layout_hash()
        ))
        .into();
        Ok(Self {
            root,
            compiler,
            salt,
            pipelines: Default::default(),
            digests: Default::default(),
        })
    }
    /// Releases every pipeline this cache made, after the last list that uses one was
    /// submitted. The cache stays usable and makes them again when asked.
    pub fn retire(&self, gpu: &mut GpuDevice) -> Result<()> {
        for (_, pipeline) in self.pipelines.borrow_mut().drain() {
            gpu.retire_pipeline(pipeline)?;
        }
        Ok(())
    }
    /// Compile only on a cache miss, then use Ara's hashed artifact/PSO loader.
    pub fn pipeline(
        &self,
        gpu: &mut GpuDevice,
        pass: Pass,
        state: RenderState,
    ) -> Result<Pipeline> {
        let source = pass.source();
        let digest = *self
            .digests
            .borrow_mut()
            .entry(pass)
            .or_insert_with(|| shader::digest(&source));
        self.pipeline_source(gpu, pass, source.as_ref(), digest, state)
    }
    /// Compile a caller-authored compute program through the same native cache.
    pub fn program(&self, gpu: &mut GpuDevice, program: &shader::Program) -> Result<Pipeline> {
        ensure!(
            !program.pass().raster(),
            "raster programs require explicit render state"
        );
        self.program_state(gpu, program, RenderState::default())
    }
    /// Compile caller-authored raster WGSL with explicit attachments and vertex state.
    pub fn program_state(
        &self,
        gpu: &mut GpuDevice,
        program: &shader::Program,
        state: RenderState,
    ) -> Result<Pipeline> {
        self.pipeline_source(
            gpu,
            program.pass(),
            program.source(),
            program.digest(),
            state,
        )
    }
    fn pipeline_source(
        &self,
        gpu: &mut GpuDevice,
        pass: Pass,
        wgsl: &str,
        digest: [u8; 32],
        state: RenderState,
    ) -> Result<Pipeline> {
        // The per-vertex families' fragment stages declare barycentrics, which Vulkan
        // has only where the device enabled the extension.
        ensure!(
            pass.metal_version() != (4, 0) || gpu.info().fragment_barycentrics,
            "{pass:?} reads per-vertex attributes, which this device does not support"
        );
        // The key covers the WGSL by its digest, its entry and state here, and through
        // the salt the adapter's sources, the compiler and the binding layout.
        let mut key = Sha256::new();
        key.update(self.salt);
        key.update(format!("{}:{state:?}:", pass.entry()));
        key.update(digest);
        let key = format!("{:x}", key.finalize());
        if let Some(kept) = self.pipelines.borrow().get(&key) {
            return Ok(*kept);
        }
        let path = self.root.join(&key);
        if !path.join("pipeline.json").exists() {
            let scratch = self
                .root
                .join(format!(".{}-{}-{}", key, std::process::id(), next()));
            std::fs::create_dir(&scratch)?;
            let result = self.emit(pass, wgsl, state, &scratch, &key);
            if let Err(e) = result {
                let _ = std::fs::remove_dir_all(&scratch);
                return Err(e);
            }
            match std::fs::rename(&scratch, &path) {
                Ok(()) => {}
                Err(_) if path.join("pipeline.json").exists() => {
                    std::fs::remove_dir_all(&scratch)?;
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&scratch);
                    return Err(e.into());
                }
            }
        }
        let meta: CookedPipeline =
            serde_json::from_slice(&std::fs::read(path.join("pipeline.json"))?)?;
        let pipeline = gpu.create_pipeline(&meta, &path)?;
        self.pipelines.borrow_mut().insert(key, pipeline);
        Ok(pipeline)
    }
    fn emit(
        &self,
        pass: Pass,
        wgsl: &str,
        state: RenderState,
        dir: &Path,
        key: &str,
    ) -> Result<()> {
        #[cfg(target_os = "macos")]
        let stages = {
            use ara_gpu::cooked::CookedMetalStage;
            let program = shader::msl_source(pass, wgsl)?;
            let source = format!("{}{}", include_str!("shader/abi.metal"), program.source);
            std::fs::write(dir.join("source.metal"), source)?;
            tool(
                "metal",
                &[
                    if pass.metal_version().0 == 4 {
                        "-std=metal4.0"
                    } else {
                        "-std=metal3.0"
                    },
                    "-fno-fast-math",
                    "-ffp-contract=off",
                    "-c",
                    "source.metal",
                    "-o",
                    "source.air",
                ],
                dir,
            )?;
            tool("metallib", &["source.air", "-o", "source.metallib"], dir)?;
            std::fs::remove_file(dir.join("source.air"))?;
            let bytes = std::fs::read(dir.join("source.metallib"))?;
            let entries: Vec<_> = match program.entries {
                shader::MetalEntries::Raster { vertex, fragment } => {
                    let mut stages = vec![(Stage::Vertex, vertex)];
                    if let Some(fragment) = fragment {
                        stages.push((Stage::Fragment, fragment));
                    }
                    stages
                }
                shader::MetalEntries::Compute { entry } => vec![(Stage::Compute, entry)],
            };
            let mut stages = Vec::new();
            for (stage, entry) in entries {
                let reflection = format!("{entry}.json");
                let value = if stage == Stage::Compute {
                    serde_json::json!({"EntryPoint":entry,"state":{"tg_size":shader::threads_source(pass, wgsl)?}})
                } else {
                    serde_json::json!({"EntryPoint":entry,"state":{}})
                };
                cooked::write_json(&dir.join(&reflection), &value)?;
                stages.push(CookedStage {
                    source_entry: entry.clone(),
                    spirv_entry: entry,
                    stage,
                    spirv: String::new(),
                    dxil: String::new(),
                    spirv_hash: String::new(),
                    dxil_hash: String::new(),
                    metal: Some(CookedMetalStage {
                        metallib: "source.metallib".into(),
                        metallib_hash: format!("{:x}", Sha256::digest(&bytes)),
                        reflection,
                        converter: format!("naga30 + {}", self.compiler),
                        root_signature_hash: cooked::binding_layout_hash(),
                    }),
                });
            }
            stages
        };
        #[cfg(not(target_os = "macos"))]
        let stages = emit_spirv_source(pass, shader::spirv_source(pass, wgsl)?, dir)?;
        cooked::write_json(
            &dir.join("pipeline.json"),
            &CookedPipeline {
                version: 1,
                key: key.into(),
                binding_layout_hash: cooked::binding_layout_hash(),
                compiler: self.compiler.clone(),
                stages,
                state,
            },
        )?;
        Ok(())
    }
}
/// Emit the portable stages for Windows validation and replay, without Slang.
pub fn emit_spirv(pass: Pass, dir: &Path) -> Result<Vec<CookedStage>> {
    emit_spirv_source(pass, shader::spirv(pass)?, dir)
}
/// Export a caller-authored program for the native Vulkan gate.
pub fn emit_spirv_program(program: &shader::Program, dir: &Path) -> Result<Vec<CookedStage>> {
    emit_spirv_source(program.pass(), program.spirv()?, dir)
}
fn emit_spirv_source(
    pass: Pass,
    source: Vec<shader::SpirvStage>,
    dir: &Path,
) -> Result<Vec<CookedStage>> {
    std::fs::create_dir_all(dir)?;
    let mut stages = Vec::new();
    for stage in source {
        let name = format!("{pass:?}-{}.spv", stage.entry);
        let bytes: Vec<_> = stage.words.iter().flat_map(|w| w.to_le_bytes()).collect();
        std::fs::write(dir.join(&name), &bytes)?;
        let kind = match stage.stage {
            naga::ShaderStage::Compute => Stage::Compute,
            naga::ShaderStage::Vertex => Stage::Vertex,
            naga::ShaderStage::Fragment => Stage::Fragment,
            _ => anyhow::bail!("unsupported shader stage"),
        };
        stages.push(CookedStage {
            source_entry: stage.entry,
            // The adapter renames the selected IR entry to the native ABI.
            spirv_entry: "main".into(),
            stage: kind,
            spirv: name,
            dxil: String::new(),
            spirv_hash: format!("{:x}", Sha256::digest(&bytes)),
            dxil_hash: String::new(),
            metal: None,
        });
    }
    Ok(stages)
}
fn next() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
#[cfg(target_os = "macos")]
fn tool(name: &str, args: &[&str], dir: &Path) -> Result<()> {
    let output = std::process::Command::new("xcrun")
        .args(["-sdk", "macosx", name])
        .args(args)
        .current_dir(dir)
        .output()?;
    ensure!(
        output.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
