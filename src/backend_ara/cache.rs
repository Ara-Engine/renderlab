//! Content-addressed native artifacts; production pipeline loading stays in ara-gpu.
use super::shader::{self, Pass};
use anyhow::Result;
#[cfg(target_os = "macos")]
use anyhow::{Context, ensure};
use ara_gpu::{
    GpuDevice, Pipeline,
    cooked::{self, CookedPipeline, CookedStage, RenderState, Stage},
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Owned native shader cache. Construct outside frame recording.
pub struct ShaderCache {
    root: PathBuf,
    compiler: String,
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
        Ok(Self { root, compiler })
    }
    /// Compile only on a cache miss, then use Ara's hashed artifact/PSO loader.
    pub fn pipeline(
        &self,
        gpu: &mut GpuDevice,
        pass: Pass,
        state: RenderState,
    ) -> Result<Pipeline> {
        let source = format!(
            "{}{}{}{}{}{}{}",
            pass.source(),
            include_str!("shader/metal.rs"),
            include_str!("shader/spirv.rs"),
            include_str!("shader/mod.rs"),
            include_str!("shader/skin_interface.rs"),
            include_str!("cache.rs"),
            include_str!("../../Cargo.lock")
        );
        let key = format!(
            "{:x}",
            Sha256::digest(format!(
                "renderlab-ara-v1:{source}:{}:{state:?}:{}:{}:{}",
                pass.entry(),
                self.compiler,
                include_str!("shader/abi.metal"),
                cooked::binding_layout_hash()
            ))
        );
        let path = self.root.join(&key);
        if !path.join("pipeline.json").exists() {
            let scratch = self
                .root
                .join(format!(".{}-{}-{}", key, std::process::id(), next()));
            std::fs::create_dir(&scratch)?;
            let result = self.emit(pass, state, &scratch, &key);
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
        Ok(gpu.create_pipeline(&meta, &path)?)
    }
    fn emit(&self, pass: Pass, state: RenderState, dir: &Path, key: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        let stages = {
            use ara_gpu::cooked::CookedMetalStage;
            let program = shader::msl(pass)?;
            let source = format!("{}{}", include_str!("shader/abi.metal"), program.source);
            std::fs::write(dir.join("source.metal"), source)?;
            tool(
                "metal",
                &[
                    "-std=metal3.0",
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
            let entries: Vec<_> = match program.vertex_entry {
                Some(vertex) => vec![(Stage::Vertex, vertex), (Stage::Fragment, program.entry)],
                None => vec![(Stage::Compute, program.entry)],
            };
            let mut stages = Vec::new();
            for (stage, entry) in entries {
                let reflection = format!("{entry}.json");
                let value = if stage == Stage::Compute {
                    serde_json::json!({"EntryPoint":entry,"state":{"tg_size":pass.threads()?}})
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
        let stages = emit_spirv(pass, dir)?;
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
    std::fs::create_dir_all(dir)?;
    let mut stages = Vec::new();
    for stage in shader::spirv(pass)? {
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
            source_entry: stage.entry.clone(),
            spirv_entry: stage.entry,
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
