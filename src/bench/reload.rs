//! The bench's rebuild for the passes outside `gi` and `vsm`: program shading, reflections,
//! transparency, the GI receiver, TR1 and primary visibility (docs/bench.md). Their kernels all
//! compose the traced heads or the prelude, so one rebuild serves them: the engine's own prelude
//! reload (`Renderer::reload` with the installed prelude), which recompiles the engine pipelines
//! and every program's modules in one transaction, then the caches keyed by the prelude's and
//! the traced kernels' hashes are invalidated and `prepare_view_pipelines` builds what the
//! fixtures' frames draw with. The new text comes from `edits` (applied as each module is
//! created); kernels whose SPIR-V did not change are driver cache hits.
use crate::Error;
use crate::renderer::Renderer;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};

/// Mixed into the hashes that key cached pipelines, so every reload misses them.
static REVISION: AtomicU64 = AtomicU64::new(0);

impl Renderer {
    pub(super) fn bench_reload_prelude(&mut self) -> Result<(), Error> {
        let slots = crate::pipeline_compilation::prepare_compile_slots();
        crate::pipeline_compilation::with_compile_slots(slots, || self.bench_rebuild_prelude())
    }
    fn bench_rebuild_prelude(&mut self) -> Result<(), Error> {
        // Composed sources are unchanged, so the module cache would return the old modules.
        self.program_cache.modules.clear();
        // The programs the last frame drew with: prepare builds their sets, hits, cell shading and
        // receivers (preparing every loaded program also built list shading no frame here draws).
        let mut programs: Vec<u64> = self.hot.iter().map(|&(id, _)| id).collect();
        programs.sort_unstable();
        programs.dedup();
        if programs.is_empty() {
            programs.push(self.standard.id());
        }
        let prelude = self.source.clone();
        self.reload(&prelude).map_err(|e| Error::Shader(e.to_string()))?;
        // The keys of the receiver's, transparency's and the visibility buffer's pipelines.
        let revision = REVISION.fetch_add(1, Ordering::Relaxed) + 1;
        let salt = |hash: [u8; 32]| -> [u8; 32] { Sha256::new().chain_update(hash).chain_update(revision.to_le_bytes()).finalize().into() };
        self.source_digest = salt(self.source_digest);
        self.primary_hash = salt(self.primary_hash);
        self.shader_hash = salt(self.shader_hash);
        self.gi_receiver_pipelines.clear();
        self.visibility_pipelines = None;
        self.bench_drop_reflection_trace();
        // Cell shading composes the program stage too.
        self.bench_drop_gi_shading();
        self.prepare_view_pipelines(&programs)?;
        self.bench_build_lean(&programs);
        Ok(())
    }
}
