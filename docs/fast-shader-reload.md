# Fast shader reload

Follow-up: the [programmable-shading API update](programmable-shading-api-gaps.md)
exposes the viewer's watcher to library callers and registers arbitrary program
handles/files. Measurements below remain the historical fast-reload campaign.

2026-11-05 (08:40–12:05 UTC), based on main `767c28c`. RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, Ryzen 7 7800X3D (8 cores, 16 threads). Release test binaries were built with two Cargo jobs, and validation was off for timing. Everything ran offscreen except two `--background` viewer runs. The scene is the registered full-resident quarry (12 draws, 13,034,903 triangles) at 1920×1080 1x on the traced-capable hardware-shadow renderer, as in [phase 1](programmable-shading-phase-1.md). Labels: **M** measured here, **I** inference.

**Result.**
- **Limiter:** the NVIDIA driver's compile of each program shading pipeline, about 1 s per pipeline. About 80% of that is the inlined environment code. naga, SPIR-V and wgpu together cost about 5 ms per module.
- **Change:** reloads still create every path's module, so naga validates the edit against each path. They now compile only the pipeline sets the last frame drew with, concurrently. Other sets compile when a frame first needs them.
- **Pipeline cache:** program pipelines go through a `wgpu::PipelineCache`, so returning to a source already compiled in the session costs about 15 ms.
- **Edit-to-image:** 2.9–5.2× faster ([below](#edit-to-image)). A program edit takes 1.5 s raster and 1.9 s traced, against 4.3 s and 10.1 s before.
- **Viewer:** it now watches `shaders/programs/standard.wgsl` (C17).
- **Output:** byte-identical.
- **Target missed:** the 100 ms target is still missed, by the driver compile of one or two pipelines. The next levers are [below](#limits-and-next-steps).

## Profile (baseline `b0`)

`gpu_reload_profile` (`src/reload_tests.rs`) composes the production sources and times each stage on the production ABIs. Every compile gets a unique edit, so no cache can serve it. Medians of 3, in ms (M). The receiver-policy worker's Cargo builds ran during 27 of the 89 one-second contention samples (at most 3 compiler processes), so these times may be somewhat inflated.

| Stage, per module or pipeline | Cost |
|---|---|
| naga parse + validate, one composed module (500–750 lines) | 1.2–2.7 |
| naga SPIR-V, whole module or per entry point | 0.3–1.3 |
| `create_shader_module` (wgpu parse, validation, whole-module SPIR-V) | 1.7–3.4 |
| Driver compile, raster lit pipeline (Lit, Hardware) | 1,300–1,500 each |
| Driver compile, fresh lit pair | 1,290–1,720 for both |
| Driver compile, traced Hard + Soft shading pair | 2,920–3,500 for both |
| All ten engine pipelines on a prelude edit (prelude 34, fresh kernels 75, traced kernels 321) | 430 |
| The same sources again in the same process, without a pipeline cache | the same cost |

Per edit, before this change:

| Path selected | Edit | Pipelines compiled | Pipelines the quarry frame draws with |
|---|---|---:|---|
| Raster | program | 6 (Lit, Hardware and Fresh pairs) | the Fresh pair |
| Traced | program | 8 (those, plus traced Hard and Soft) | traced Soft |
| Raster | prelude | 6 + 6 engine | the Fresh pair + fresh kernels |
| Traced | prelude | 8 + 10 engine | traced Soft + trace and fresh kernels |

**Where the driver spends a compile** (ablations of the standard program, one Lit pair and one traced pair, medians of 3, M):

| Standard program variant | Lit pair | Traced pair |
|---|---:|---:|
| Unchanged | 2,751 | 3,059 |
| Environment branch disabled (`if false`) | 528 | 616 |
| Light loop removed | 2,331 | 2,758 |
| Both | 473 | 543 |
| Flat (returns the base factor) | 22 | 58 |

The environment branch is about 80% of the compile. `environment_radiance` inlines `environment_level` five times (two legacy levels, one rough level, two blended levels), and each `environment_level` inlines three `environment_bilinear` calls of four taps, each with corner remapping. That makes 15 inlined bilinear filters before the irradiance lookup (I, from the source structure). The light loop costs about 15%.

**The 6–15% slower program modules.** The program API adds about 14 functions and 14 calls to every lit entry point, and passes `SurfaceInput` (which embeds the 48-byte `Material`) by value. Fragment and traced entry SPIR-V grows by 7–12% against the frozen pre-program compositions (M):

| Entry point | Frozen words / functions / calls | Program words / functions / calls | Words | Phase 1 compile ratio |
|---|---|---|---:|---:|
| Raster `fs_lit` | 7,927 / 15 / 42 | 8,750 / 29 / 56 | +10.4% | 1.148 |
| Hardware `fs_lit` | 8,490 / 18 / 44 | 9,317 / 32 / 58 | +9.7% | 1.105 |
| Fresh `fs_lit` | 6,592 / 15 / 41 | 7,352 / 29 / 55 | +11.5% | 1.126 |
| Traced Hard / Soft `primary_shade_tiles` (words) | 12,922 / 11,025 | 13,880 / 11,929 | +7.4% / +8.2% | 1.060 (pair) |

The compile difference tracks the SPIR-V growth and is smallest on the traced pair. The cause is very likely the API layer's call boundaries and by-value struct traffic, which the driver must inline and scalarise (I). It is small next to the environment code. Shrinking it would change the program API (for example, passing the receiver by pointer), which this unit leaves alone.

## What changed

- **Program pipelines are created on first use** (`src/program_gpu.rs`). Each program keeps a module for every path the renderer has: raster lit, hardware, fresh, and traced Hard and Soft while traced is selected. It gets a pipeline set per `Variant` (Lit, Hardware, Fresh, Traced) once something needs one.
  - Frame admission (`Renderer::frame_paths`, shared with `submit_frame`) names the set the frame draws with. After every admission check and before the frame's uploads, `ensure_pipelines` creates the sets its programs lack. The virtual path does the same before submission. Unlit frames need none.
  - The sets the last frame drew with are the *hot* sets. A program reload, or the program half of a prelude reload, recreates exactly those inside the transaction, so the next frame compiles nothing. The rest are dropped and compile at first use.
  - Renderer creation, `try_create_program` and traced selection create modules only. Traced selection still compiles the four traced kernels. Renderer construction on the test renderer fell to 55 ms (`all_renderer_pipeline_setup_ms`); the first lit frame now pays for its own set.
- **Concurrent creation.** Every pipeline of a reload, or of a frame's missing sets, is created on its own scoped thread (`in_parallel`). Each thread has its own error scopes, because wgpu error scopes are per thread. A prelude reload also compiles the engine pipelines on a thread of their own, concurrently with the programs. `FreshState::shade_pipeline` and `PrimaryState::shade_pipeline` each create one pipeline of a pair.
- **Pipeline cache** (`program_gpu::ProgramCache`).
  - Program pipelines go through one `wgpu::PipelineCache` when the device enables `Features::PIPELINE_CACHE`. The viewer requests the feature when the adapter has it.
  - The cache is replaced, dropping its history, after 512 pipelines. That bounds it to roughly 50 MB at the measured 93 KB per lit pipeline.
  - Engine kernels stay uncached; they cost 0.2–0.4 s in total.
- **Transactions reject the same edits as before.**
  - Every path's module is created in the reload, so naga's parse, type and stage validation runs against each composition. A traced-only error such as implicit sampling is still rejected at its program line while traced is selected (`gpu_reload_on_demand`).
  - One kind of error now surfaces later: a pipeline-creation error on a path no frame is drawing with, for example a program that reads an engine binding missing from one path's layout. It fails the first frame that needs that set, with `Error::Shader` naming the program and set, before that frame's uploads. Other paths keep rendering.
  - Before this change, a traced-only error already surfaced late, at traced selection.
- **The viewer watches the standard program** (`viewer/src/shader_watch.rs`, C17).
  - It polls `scene.wgsl` and the `programs/standard.wgsl` beside it every 50 ms, by content. `--program` overrides that path.
  - Each changed file reloads in its own transaction, programs first.
  - A rejected file is retried after the next successful reload of the other file, so a program that needs a new prelude helper applies once the helper lands.
  - The shader-write marker now prints `save_to_detect_ms` (the file's age when read, bounded by the poll) and `detect_to_<event>_ms` separately.
- `PipelineStats::created` counts pipelines when they are actually created.

## Mechanisms screened

| Mechanism | Screen (M) | Decision |
|---|---|---|
| Compile only the sets the frame draws with | Program edits drop from 6 pipelines (raster selected) or 8 (traced) to the 2 of the hot set | Retained |
| Concurrent pipeline creation | Profile: two pipelines from one module finished together (1,347 ms) at about the single-pipeline time. Later screens: two concurrent independent pipelines took 1.5–2.1 s each, against about 1.0 s alone in mech-b2, so this driver scales concurrent compiles only partly | Retained, one thread per pipeline. Fewer pipelines is the stronger lever |
| `wgpu::PipelineCache` (`gpu_reload_mechanisms`: two concurrent independent lit pipelines, then again from fresh modules, n=3) | First compile: no cache 1,655 / 2,011 / 2,141 ms, one shared cache 1,510 / 1,491 / 1,651, one cache each 1,593 / 1,545 / 2,016. Repeat: no cache 1,584 / 2,209 / 1,962, either cache 7–9 | Retained (shared). No first-compile cost and no serialisation of concurrent compiles |
| Per-entry-point SPIR-V instead of whole-module (forced with an unused override, which wgpu-hal compacts per entry) | Paired, alternating, n=4: Lit pair 1,788 against 2,051 ms, Hardware 1,940 against 1,865, traced 4,094 against 4,128 | Rejected: no consistent difference |
| Two-level environment probe: `environment_level` inlined twice instead of five times, with the same arithmetic per path (test-only edit of the prelude) | Prelude reload with the hot set, n=2 each: raster 1,909 → 1,211 ms (−37%), traced 2,377 → 1,382 ms (−42%). HDR byte-identical on both paths, as is the uniquified production control | Not applied: shaders are outside this unit's ownership. It is the next lever |
| Asynchronous creation off the frame thread | Not built. It would keep the viewer presenting during a compile, but would not shorten edit-to-image. It needs `'static` copies of the shading layouts held by the fresh and traced GPU state | Deferred |

**Driver caches** (M): NVIDIA's own disk cache serves SPIR-V that an earlier process compiled, but not SPIR-V first compiled in the current process. That is why the profile's same-process repeats cost the full compile. It is also why early cache screens that reused a fixed sequence of edit values were served from disk. Every measured edit now takes a per-process random value (`salt` in `src/reload_tests.rs`).

## Edit-to-image

`gpu_reload_edit_to_image` drives the viewer's own `ShaderWatch` on the quarry.
- It saves a tint edit to a scratch copy of `scene.wgsl` or `programs/standard.wgsl`, and the poll runs 50 ms apart at a phase spread over the interval.
- Each sample measures save → detection (the poll wait) → reload → one frame rendered and completed on the GPU. Presentation is excluded.
- Each edit uses a tint unique to its process, so no cache can serve it. The revert restores the original text, and every revert image is byte-identical to the first image.
- The baseline is `767c28c` with only the test files added (`b0g`). The candidate is `b4`.
- Runs alternated by build. Medians [min, max] of 5, in ms.

| Path | Edit | Before (`b0g-1`) | After (`b4-2`) | Speed-up | Pipelines before → after |
|---|---|---:|---:|---:|---|
| Raster | program | 4,310 [4,198, 4,460] | 1,476 [1,427, 1,900] | 2.9× | 6 → 2 |
| Raster | prelude | 6,328 [4,949, 6,917] | 1,552 [1,520, 2,165] | 4.1× | 12 → 8 |
| Traced | program | 10,090 [8,987, 11,548] | 1,928 [1,904, 2,120] | 5.2× | 8 → 2 |
| Traced | prelude | 10,458 [9,583, 11,703] | 2,455 [2,024, 3,034] | 4.3× | 18 → 12 |

- **Contention.** These two runs had compilers in 0 of 66 and 2 of 16 one-second samples. The other two runs ran during another project's builds:
  - `b0g-2`, compilers in 19 of 38 samples: raster program 4,375, raster prelude 4,604, traced program 8,952, traced prelude 12,108;
  - `b4-1`, compilers in 6 of 17 samples: raster program 2,034, raster prelude 1,870, traced program 2,758, traced prelude 4,356.
- **Save to detection** is the poll wait: medians 18–33 ms, at most 48 ms. Detection to image is the rest; for example, 1,441 ms for the raster program edit after the change.
- **Reverts** take 79–127 ms on raster and 93–188 ms on traced after the change, served from the pipeline cache.
  - Before the change, raster reverts already took 55–110 ms, from NVIDIA's disk cache of the original sources.
  - Traced reverts before took 4,033 and 4,647 ms in `b0g-1`, then 73 and 104 ms in `b0g-2` once that run had populated the disk cache.
  - The pipeline cache makes in-process repeats a hit regardless of the driver's disk cache.
- **Rejections.** Invalid program and prelude saves were rejected through the watcher in 0.4–4.8 ms, and the accepted image was kept on both paths in every run.

**Viewer** (background window, plain raster renderer, quarry, lit, 1x; the script saved the same tint and its revert five times per file):

| Build | File | First edit (a real compile) | Repeated edits and reverts |
|---|---|---:|---|
| Before `v0` | `scene.wgsl` (the only watched file) | 1,859–2,444 (every save recompiles) | edits 1,859–2,444 (median 2,344), reverts 1,813–2,600 (median 2,046) |
| After `v3` | `programs/standard.wgsl` | 1,675 | 27–59 (pipeline cache) |
| After `v3` | `scene.wgsl` | 1,696 | 61–120 |

The viewer's split is about 15–33 ms save to detection, then detection to completion. Startup to the first completed frame was 9,229 against 8,619 ms (one sample each, including the asset upload).

## Equivalence and checks

- **Traced primary acceptance** (`gpu_primary_traced_quarry_acceptance`, validation on): 285 of 285 image, count and receiver files byte-identical between the baseline and the candidate, including the raster and raster-hard images. Validation reported 0 errors in both runs. The accepted `accept-b12` count files had been removed from D: (the folder changed at 10:45 UTC), so the baseline run regenerated them.
- **Determinism static gate** (`scripts/determinism-run.py --test static`, 32 repeats, Core and Synchronization validation): 3 of 3 processes pass, 1,152 identical static comparisons.
  - Each process's 36 complete captures match all 36 hashes that `gate-main-final` recorded for `accept-c11`. Those reference files have also been trimmed to their HDR images.
  - The quarry replay counts the test reads (`m6-level-b-probe/runs/quarry-b6/*-1x8.u32`) are gone. So the gate binary `b4v` adds one uncommitted test-side fallback that skips that comparison when its file is missing. Production code is identical to `b4`. The Interactive counts the replay check covered are among the 36 matched hashes.
- **GPU tests, validation on, 0 errors:** `gpu_reload_on_demand`, `gpu_program_api_raster_and_traced`, `gpu_main_depth_order_and_reload`, `gpu_images_uploads_reload_and_timing`, `gpu_lit_photometry_1x_4x`, `gpu_lit_edits_and_bundle_recovery` and the five `hardware_shadow_tests`.
  - Two failed on the first candidate. The hardware rejection test saw the frame's Lit pair created before a later admission check rejected the frame, so ensure moved after `prepare_scene`'s admission. The lighting busy-frame helper asserted that a first lit frame creates no pipelines, so it now creates the standard sets first.
- **CPU:** 81 library tests pass. Workspace Clippy `--all-targets -D warnings`, `cargo fmt --check` and `git diff --check` pass.
- **Not run:** the 18 raster lit suites, the thin and stress replays and the film reference images. They exercise no reload path, and output equality is covered by the two acceptance sets above.

## Limits and next steps

- **The 100 ms target is missed.** After this change, an edit costs the driver compile of the frame's set: two pipelines, about 1.4–1.9 s concurrently on the quarry's Fresh and traced sets.
  - **Environment helpers** (the largest lever): the two-level probe saves 37–42% and is byte-identical on both paths. A rewrite that filters once per level, choosing the texture at each load, should save more (I, unmeasured). It belongs to whoever owns `scene.wgsl`.
  - **Per-pipeline sets.** The quarry draws one pipeline of each pair: tangent meshes plus the standard non-tangent pipeline the pass binds first on raster, and Soft only on traced. Creating single pipelines would skip about half of each set. It needs per-pipeline storage, and the `test_standard` pair swaps in other test files would change.
  - **Asynchronous creation** would keep the viewer presenting during the compile without shortening edit-to-image.
- **Pipeline cache:**
  - it is in-process and bounded, and is not persisted, because on this machine NVIDIA's disk cache covers cross-process repeats; drivers without one would need persistence;
  - the caller must enable `Features::PIPELINE_CACHE`, which the test helper in `fresh_shadow_tests.rs` does not by default; the new tests pass it.
- **Late pipeline errors:** pipeline errors on cold sets surface at first use, not at reload (see [what changed](#what-changed)).
- **Measurement base:** measurements are on `767c28c`. The receiver-policy branch (`22681051`) changes `scene.wgsl` and the shadow shaders, so compile costs shift after that merge. Its `renderer.rs` hunks (device-feature admission, the material thin initializer, the shadow-depth `CompareFunction::Greater`) don't overlap this change's.
- **Viewer coverage:** the viewer has no hardware or traced paths. Those paths were measured offscreen through the viewer's own `ShaderWatch`.

## Evidence and time

Evidence is in `E:/renderlab-work/fast-reload/`:
- the scripts `run.py`, `batch.py`, `wait.py`, `build.py`, `build_viewer.py` and `viewer_run.py`;
- `tools/` (edit scripts, `summarize.py`), `manifests/` and `runs/` (logs, JSON results, hashes, `primary-hashes.json`);
- `builds/`, where each build has its SHA-256, HEAD and source diff;
- `gpu-windows.json` and `gpu-schedule.md`, the slot coordination with the receiver-policy worker;
- the baseline worktree `base/`: `767c28c` plus the test files only, detached, with its own `target-base`. A shared target let cargo reuse one tree's units for the other, so the first viewer and baseline builds from it were discarded.

Time:
- **Verification:**
  - 40 GPU-slot holds totalling 2,543.5 s, the longest 298.7 s, all released, the last at 11:59:47 UTC. Every preflight found no game, Unreal Editor or foreign renderlab GPU job and at least 6 GiB available. Deferrals for memory (other projects' builds) or a held slot were waited out.
  - 15 release builds took 684 s. 37 offscreen test children took 1,162 s, plus 5 determinism-runner processes (2 stopped early on missing reference files) and 2 viewer runs (5 min).
  - CPU checks took about 2 min.
- **Waiting:** about 105 min went to the receiver-policy worker's GPU windows. Coding continued during them.
- **Implementation** (reading, profiling design, code, docs): the rest of the 3 h 25 min, interleaved with verification and not separately instrumented.
