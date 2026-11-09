# M7 ARA1: measured Ara / Renderlab comparison

Phase 2, 2026-11-09, branch `t3/m7-ara1`. **Renderlab has lower displayed-image error against the matched references in all three opaque scenes; performance is mixed and non-RT is substantially slower.** RT is faster on the quarry and slower on the smaller scenes. New-shader latency remains a 23–24.5 s hitch. Prioritize those latency costs and the missing particle/animated-character fixtures next. Same-model references and incomplete feature coverage prevent a universal quality/speed or shipping-parity certificate.

Visible misses: both Renderlab paths show a box-shaped mirror-sun glint during the late pan (frame 71); mirror-crop RMS rises from about 0.042 still to 0.069 in that view. Alpha-panel ground-shadow response and sky/horizon distributions differ between engines. GI-enabled game variants also have roughly 21–31 ms RT first-cut service versus about 2–4 ms for Ara hybrid. These remain gaps even where the aggregate reference error is lower. The page includes per-aspect late-motion/cut error verdicts and crops.

Local self-contained page: `D:/Coding/renderlab-work/ara1/report/index.html` (embedded lossless images and provenance). `inline.html` is the small local view used to show the same content in T3; `provenance.json` and `verdicts.json` are alongside. Root owns publication; nothing was uploaded.

## Frozen sources and context

- Renderlab renderer: **`ae857501ab4792d1889e11111064c3957cc8af44`**, merged into this comparison branch at `6d2d8957965b1aafb7cced0eefa1ceb179923577`. `src/`, `shaders/`, `crates/` and `viewer/` match that final base. Earlier a2c92b1 Renderlab captures are superseded. Only measurement adapters/tools/docs changed here.
- Ara shipping main: `1d2d647f0074f9f1f1d7f69a86f6d71b3142c9e3`, read-only checkout `D:/Coding/renderlab-work/ara1/ara-main`. Ara render-v3 `1e04fe5dd9d04ede28592597dace9c0c06a15f9a` was inventoried/preparation-typechecked, **not GPU measured** or substituted for shipping main. No Ara worktree files were edited, committed or pushed.
- RTX 4070 SUPER, 12,282 MiB reported VRAM, NVIDIA 617.14; Ryzen 7 7800X3D, 8 cores / 16 threads; Windows. Optimized release profiles, Renderlab Rust 1.96 and Ara Rust 1.99.1. Native resolution: quality/reference **960×540**, timings **1920×1080**. No stock-upscaling or Unreal comparison.
- Each capture records binary/source/harness/scene/asset hashes, settings, resolution, hardware, cache state, lease/order and contention. Root assigned the sole GPU to ARA1 for final work. Primary timing samples ran without parallel ARA1 builds/scoring; some feature leases overlapped CPU image scoring. Ordinary OS activity remains. See embedded per-run provenance rather than treating these values as portable engine constants.

## Matched content

| Scene | What is shared | Correction / limit |
| --- | --- | --- |
| Frozen game | Same RLA geometry/materials/placements from `examples/game_scene.rs`, camera, sun/sky and exposure | Standard PBR replaces toon hero; no decals, rigid animation, VFX or skinning. Alpha, 64 lights, atmosphere, fog and bloom are separate named variants. |
| Ara PBR | Original common-sample sphere grid and Avocado, texture/tangent data, factors, camera and lighting | Shared canonical winding: 22,083 triangles flipped against authored normals, 1,641 kept. CPU roundtrip is byte-idempotent. Original inward procedural winding produced a false brightness discrepancy and is rejected from primary verdicts. Cyan emissive/blended spheres and studio HDR are excluded. |
| Quarry | Original full-detail private RLA, same camera, neutral sky/sun | A common **−8.5 stop** exposure replaces the nearly black inherited HDR-sky exposure after uniform-sky substitution. No per-engine exposure fit or decimation. Original dark case is superseded. |

Both native engines consume the same RLA bytes and mip chains. Ara fidelity's public API flattens instances and regenerates mips from level zero; keep its startup/memory/filtering differences separate. Uniform environment, 0.00465-rad sun, 0.1 m near plane, fixed exposure, ACES-fit and sRGB are shared. Ara's infinite far plane versus Renderlab's 10 km far plane preserves camera rays. Ara High shading budgets at native resolution are a controlled profile, not stock High's 0.6-axis reconstruction.

## Per-aspect verdicts

The numerical better/worse statements are **displayed linear-RGB RMS on preregistered game crops**, not isolated algorithm scores or all-scene shipping certificates. They include shading/filtering/history differences. The same-model reference bias and split-half uncertainty apply; a descriptive 2% band is not a statistical confidence interval.

| Aspect | RL RT vs Ara hybrid | RL non-RT vs Ara raster |
| --- | --- | --- |
| GI / bounce | Better: 55.1% lower error (0.01644 / 0.03661) | Better: 45.5% lower error (0.01999 / 0.03669) |
| Sun shadows | Better: 57.1% lower error (0.02058 / 0.04803) | Better: 70.4% lower error (0.02169 / 0.07317) |
| Mirror reflections | Better: 61.4% lower error (0.04215 / 0.10918) | Better: 70.0% lower error (0.04290 / 0.14310) |
| Glossy reflections | Better: 56.5% lower error (0.03586 / 0.08241) | Better: 68.1% lower error (0.03719 / 0.11640) |
| AA / temporal stability | Equal within conservative reference-noise bound (0.00638 / 0.00962; noise 0.00259) | Equal within conservative reference-noise bound (0.00650 / 0.00963; noise 0.00259) |
| Local shadows / many lights | Larger configured coverage: all 64 lights eligible for RT visibility | Larger budget: up to 64 atlas lights versus Ara High’s four |
| Transparency | Matched alpha panel; quality unranked | Matched alpha panel; quality unranked |
| Particles | Unmatched | Unmatched |
| Sky / fog / atmosphere | Matched controls; quality unranked | Matched controls; quality unranked |
| Post processing | ACES-fit matched; bloom unranked | ACES-fit matched; bloom unranked |
| Skinned characters | Unmatched | Unmatched |

## Frame time and memory

Single frame in flight. CPU service includes submission plus GPU drain, excludes image readback/PNG. Still/moving/recovered medians use eight samples each; first cut is one observation. Ara GPU pass sums are not an end-to-end span. Full per-pass still/move/cut costs, distributions and per-feature ablations are in the page/provenance.

| Scene | Renderer | Still ms | Moving ms | First cut ms | Recovered ms | GPU span ms | Dedicated MiB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| game-base | Ara RT fidelity | 4.820 | 4.716 | 4.394 | 4.570 | unavailable | 1218.2 |
| game-base | Ara hybrid | 2.236 | 2.348 | 2.380 | 2.225 | unavailable | 633.9 |
| game-base | RL non-RT | 10.777 | 11.829 | 12.299 | 9.168 | 9.886 | 1439.8 |
| game-base | Ara raster | 1.862 | 1.958 | 1.842 | 1.874 | unavailable | 618.7 |
| game-base | RL RT | 4.009 | 6.035 | 21.427 | 5.750 | 3.503 | 1136.4 † |
| ara-pbr-oriented | Ara RT fidelity | 4.055 | 3.757 | 3.681 | 3.808 | unavailable | 1288.6 |
| ara-pbr-oriented | Ara hybrid | 1.945 | 1.997 | 2.184 | 1.973 | unavailable | 749.1 |
| ara-pbr-oriented | RL non-RT | 7.458 | 7.681 | 8.886 | 7.542 | 6.600 | 1379.8 |
| ara-pbr-oriented | Ara raster | 1.700 | 1.712 | 1.707 | 1.611 | unavailable | 651.2 † |
| ara-pbr-oriented | RL RT | 2.928 | 3.792 | 12.242 | 3.899 | 2.287 | 1204.4 † |
| quarry-daylight | Ara RT fidelity | 4.377 | 3.936 | 4.088 | 4.261 | unavailable | 4759.7 |
| quarry-daylight | Ara hybrid | 5.749 | 8.898 | 9.133 | 5.829 | unavailable | 3388.2 |
| quarry-daylight | RL non-RT | 12.320 | 12.143 | 18.184 | 13.224 | 11.139 | 4480.7 |
| quarry-daylight | Ara raster | 5.600 | 8.706 | 8.839 | 5.575 | unavailable | 1965.8 |
| quarry-daylight | RL RT | 3.236 | 4.021 | 15.561 | 4.585 | 2.642 | 3380.9 |

VRAM is sampled per-process WDDM Dedicated Usage, separate from board memory and allocator estimates. Missing/zero short-process counters remain unavailable. Startup working-set and peak process-commit observations are separate from GPU memory; the page records each engine/case peak. The RT quarry path can be faster while the small scenes are slower; non-RT is substantially slower than Ara raster. No 2× overall performance gate is claimed.

† Memory-only supplemental runs keep short processes alive with 2,048 stationary tail frames after the same first 88 poses/settings. Their sampled memory is marked separately and does not replace any timing/startup observation. This is synchronous one-frame latency, not pipelined throughput or presented FPS. Ara native deduplicates keyed geometry; Renderlab uses the prepared-asset loader, so startup/memory describe these adapter paths.

## Startup and editing

Fresh per-run application-cache directories with a warm driver are not clean-driver cold starts. Startup is process launch to first readback, not scanout. Ara's earliest image can be a loading fallback; its configured-image time waits for GI/RT readiness. RT acquisition/baking is a separately charged preparation step, even for Ara's non-RT raster runtime.

| Renderer | Warm n | Warm startup median s | Startup range s | First WGSL edit→image s | Cached repeats ms |
| --- | --- | --- | --- | --- | --- |
| RL non-RT | 3 | 0.599 | 0.568–0.635 | 24.501 | 109.3–124.7 |
| RL RT | 3 | 0.512 | 0.505–0.535 | 23.249 | 124.5–126.5 |
| Ara raster | 3 | 1.215 | 1.183–1.301 | unavailable | unavailable |
| Ara hybrid | 3 | 1.245 | 1.239–1.273 | unavailable | unavailable |

The WGSL edit visibly reduces the live standard material's red output by 17%. **The first new edit still hitches for about 23–24.5 s; cached repeats must not hide it in a median.** Ara shader edit-to-image is unavailable under the read-only policy; cached startup is not an edit measurement.

| Engine | Rust adapter edit→image s | Scope |
| --- | --- | --- |
| renderlab | 34.534 | optimized Cargo rebuild of staged measurement adapter; renderer source unchanged; output exposure +0.125 stops |
| ara | 19.300 | optimized Cargo rebuild of staged measurement adapter; renderer source unchanged; output exposure +0.125 stops |

Rust rows are real staged exposure edits (+0.125 stops), warm Cargo rebuild/relink, restart and fresh image. They are **adapter proxies**, not core-renderer source edit measurements or interactive viewer/scanout latency. All staged source is under the evidence root; Ara renderer source is unchanged.

## Reference and audit limits

The existing plain tracer uses 16,384 spp for game and 4,096 for corrected PBR/quarry, NEE/MIS/Russian roulette, depth limit 1,024, no reuse/denoising. References cover consecutive motion and cut witnesses. Zero invalid pixels is necessary, not a convergence certificate. Rare bright paths keep finite-reference noise visible even at 16k. Existing independent Mitsuba transport cross-checks are secondary validation; this campaign uses Renderlab's standard Lambert/GGX material model and therefore has same-model bias. No old HDR quarry reference is reused.

GI/reflection-off references are reused only after exact physical/display identity checks that exclude documentary fields and those estimator switches; source process/image identities remain in sidecars. Changes to camera, size, sun, media or other physical controls fail the CPU reuse contract. One high-sample batch was rejected when a harness-test edit changed whole-checkout status; it was rerun successfully with renderer/build-input checks. Rejected data/aliases are excluded. Ara retains an entire-checkout mutation check.

Initial validation-instrumented logs contain Renderlab SPIR-V layout diagnostics. Primary game quality was recaptured without instrumentation; corrected scenes, features and performance use recorded uninstrumented release environments. No renderer fix or validation-clean certificate is claimed.

## Gaps

- Particles and animated skinned characters: both engines have counterparts, but no common runtime host/animated fixture was ported. Frozen hero geometry does not exercise either feature. This is unmeasured coverage, not evidence a feature is absent.
- Refractive/multilayer glass, physical atmosphere/fog and bloom accuracy: controls/images/timings are matched where stated, but no qualified effect reference or calibrated look-equivalence exists. Do not force a quality winner from different models.
- Local shadows: compare actual configured budgets (RL up to 128 maps / all contributing RT lights; Ara High four 512-face maps). Larger eligibility is not a readback of atlas residency or a calibrated visibility score. All 64 lights are present on both sides.
- Genuine Ara shader and core-source Rust edit loops; native HDR radiometric scores; stock upscaling; full authored Ara worlds; moving lights/geometry beyond camera motion; independent full-scene material-model references remain open.
- Historical `D:/AraBuild/render-v3/{README.md,B3-report.md,gates/G1-G6}` evidence is unavailable. Reports/source in Ara are secondary context, not reused current numbers.

## Execution and reproduction

Root's held startup window completed and the exact `m7-ara1 HELD WINDOW done` line was appended. Admission required 8 GiB available for held startup; the 2 GiB runtime guard remained active. Subsequent leases are capped at **10 minutes**, reread the latest anchored root order and poll in-process at 50 ms. Every Ara invocation checks for existing Ara/snengine renderer processes. All output is headless, under `D:/Coding/renderlab-work/ara1/`; no foreground windows, desktop/game captures or input automation.

Key CPU tools: `tools/ara1/{prepare,matrix,reference_alias,analyze,report}.py`; engine wrappers `run-renderlab.py` and `run-ara.py`; `warm.py`, `stage_loop.py` and `devloop.py` for development loops. `build.py ... --release --jobs 1` places targets under the evidence root. `ara1_prepare --orient SOURCE.rla OUTPUT.rla` makes the canonical sample before `matrix.py prepare`; `matrix.py freeze` snapshots binaries before further staging builds. Captures refuse overwrite: use new tags. After a fresh `lease.py order`, invoke the matrix only through `lease.py --expected-order HASH --label UNIQUE --minutes 10 --handoff-seconds 600 -- ...`.

`memory.py` is the leased, resource-only replay for the seven short-process WDDM gaps; it reuses successful qualifications and checks original settings/poses. Its retained-memory values are marked † and stored separately from timing observations. The PBR reference subset uses sun plus uniform sky and omits the authored sample&#39;s three local lights on both engines, because the shared reference API supports sun/sky; the separate 64-light fixtures exercise local lighting.

Final aggregation: `analyze.py --tag final --extra-tag final-clean`, then `report.py`. The scorer reuses `scripts/scan-quality-compare.py` and `support/fresh_level_b_metrics.py`; composition is CPU only. Seven focused CPU contract tests pass, including strict reference identity and physical-reuse rejection; optimized adapters ran end to end on GPU and canonical orientation is byte-idempotent. Render-v3 preparation checks are not new GPU verification.

Own Cargo targets remain at **4.42 GiB**: automatic approval review rejected recursive cleanup, including a verified literal target, with “blocked by policy.” No target was removed. Exact paths/sizes/status are in `D:/Coding/renderlab-work/ara1/target-sizes-phase2.json`. This is a cleanup gap. Frozen executables, prepared assets, bake inputs, small summaries and inspected PNGs remain; no disposable raw measurement dump over 5 MiB was found in capture/lease/CPU-verification outputs. Ara worktrees and pre-existing files remain untouched.
