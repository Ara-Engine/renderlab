# Lean shading

One lean variant of each program's traced shading module, for frames that project no decals and have no media (no aerial perspective, height fog or volumetric fog). `program::lean_variant` inserts an early return at the head of `decal_prepare` and `atmosphere_surface`, so the compiler drops the idle decal and atmosphere code. Nothing else changes: the lean module's text is the full module's text plus those two lines (`lean_modules_differ_only_by_their_early_returns`), and its picture is byte-identical. This is one variant, not a lattice of feature switches. `RENDERLAB_LEAN=0` keeps every frame on the full module.

**Decision record.** Idle feature code in the traced shading kernel costs time even when it returns at once. Decals cost 0.25 ms on the M5 in scenes with none, and the lean module that compiles it out is byte-identical. This is the second such case after the debug path's +12%.

## The two backends

- **ara-gpu port** (`backend_ara/forward.rs`, `primary_shade.rs`): the frame picks lean or full before it composes, and the program memo keys on it. It measured −0.58 ms on the M5, with an exact picture.
- **wgpu** (`lean_gpu.rs`): each program keeps its full pipelines and gains `ProgramPipelines::lean`, tiles then list, by `Shading`. Streamed-texture and temporal frames always shade full.

## Compiling lean on wgpu, and frames in the meantime

The full pipelines are built exactly as before, so cold start and time to first frame are unchanged. On the first lean frame that shades with a program lacking its lean pipeline, `prepare_lean` starts one compile on a worker thread. It uses the full pipeline's layout and the pipeline cache. Only one lean compile runs at a time; the next one starts on a later frame, after this one lands.

**A frame rendered while lean is still compiling is the full module's frame, byte for byte.** Until the lean pipeline is installed, `lean_pipeline` returns none and the shading pass binds the full pipeline the frame would have used without this change. The swap happens between frames, never within one, and the two pipelines produce identical bytes for the same frame (`gpu_lean_shading_lands_in_the_background_and_matches_full`). So there is no visible switch.

Two cases discard work instead of installing it:

- A program reload, or a change of primary visibility, while a compile is in flight discards that compile, because the program source or shading head digest no longer matches. The new source gets its own compile.
- A lean compile that fails validation is never retried. That program keeps shading full.

Reloads and visibility changes drop the lean pipelines along with the traced ones.

## Not here yet

- Composing with the reflection pass's list head: once it reaches main, merge main and re-run the byte identity with the pass on.
- RTX rows: Showcase first view on both paths, lean against `RENDERLAB_LEAN=0`, three interleaved rounds, pictures byte-compared, and the cold-compile row (time to first frame and peak process memory, with a cleared shader cache, lean on and off).
