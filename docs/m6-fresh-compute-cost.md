# Fresh compute shadow cost hillclimb

One exact mechanism is retained: each receiver's 256 rays are split across eight lanes of a 32-lane workgroup that covers a 2x2 receiver tile, and each ray stops at its first hit when `tmin > 0`. On the actual quarry, the visibility dispatch drops to **0.46-0.53 of the frozen baseline** (paired, four processes). The complete GPU span drops to **0.61-0.68**. Visibility counts, HDR and depth stay bit-identical to the fragment reference and to the [previously verified evidence](m6-fresh-compute-verification.md). This is progress, not a milestone pass. The complete span (14.6-22.6 ms process medians) remains above the historical hard-4x interval (9.5-9.8 ms) and far from Unreal-equivalent 2x performance.

## Limiter

Diagnostics ran on the real quarry receivers in one process, as paired compute-only dispatches against the frozen baseline (`support/fresh_screen_baseline.wgsl`). Ratios are per-repetition medians. The identical-source control spans 0.85-1.11, so single paired samples carry about ±10% noise.

| Probe | Paired ratio | Reading |
|---|---:|---|
| No `rayQueryProceed` (ALU, loads, loop only) | 0.056 | Traversal is about 94% of the dispatch |
| 128 instead of 256 rays | 0.451 | Cost is linear in rays |
| Rays capped at 25 cm | 0.917 | Most traversal cost is near the receiver |
| Group order swizzled into 4- or 16-group strips | 0.99-1.01 | L2 order is not the limiter |
| Other 64-256-thread groups (32x2, 16x8, 16x16; base is 8x8) | 1.08-1.15 | |
| 32-thread groups (8x4, 4x8); 16-thread (4x4, 8x2) | 0.58-0.62; 0.59-0.64 | Group size, not warp tile or lane count |
| 8x4 groups plus 4-16 KiB workgroup padding | 1.03-1.12 | Forcing a shared-memory carveout erases the gain |
| All lanes share one direction sequence (8x8; 8x4) | 0.68; 0.99 of 8x4 | Direction coherence matters only with large groups |
| Eight lanes per receiver, same origin (`ppx8`) | 0.464 | Shared origins help further |

Lane utilization of active warps is already 97.9% (CPU analysis of saved coverage). Only 135 of 5,588 active 8x8 groups pair an empty warp with an active one, so static coverage imbalance does not explain the group-size effect.

The named limiter is **BVH-traversal memory locality**: the cache-resident traversal working set of the rays in flight per SM. The evidence is that the cost tracks group size and the L1/shared carveout, while lane counts, ALU, L2 order and coverage do not. No hardware counters were available (no Nsight install), so this is an inference from controlled probes, not a counter measurement.

## Retained change

`shaders/fresh_shadows.wgsl` now dispatches `@workgroup_size(32)` groups over `FRESH_TILE` 2x2 receiver tiles. Lane `l` of a receiver traces directions `l, l+8, ..., l+248`, so each receiver's ray set is unchanged. Lanes sum their counts in workgroup atomics. Every lane of a receiver takes the same early exits, so lane zero's sentinel decides the unchanged `256` uncovered/out-of-cascade output.

Rays use `RAY_FLAG_FORCE_OPAQUE` and add `RAY_FLAG_TERMINATE_ON_FIRST_HIT` only when `tmin > 0`. The reference counts a closest hit at exactly `t = 0` as visible. `depth_bias` may be zero, so first-hit termination would not be exact there. `FORCE_OPAQUE` is a no-op for these opaque BLASes.

`fresh_shadow_gpu::TILE` sets the dispatch grid. A CPU test checks it against the WGSL constant and the naga-reflected `[32, 1, 1]` group size. The first-hit flag alone is inside noise (paired 0.92-1.00 on 8x8 groups, 0.97 on `ppx8`); it is retained for semantics without a separate speed claim.

## Equivalence and images

All gates are unchanged and ran on the final binary with active Khronos validation (`errors=0 missing_layer=0 active_layer=1`):

- The thin/contact fixture passes at five X offsets (0, 0.01, 0.02, 0, -0.01). Partial-shadow populations are 897/752/897/897/752. HDR, counts and depth winners are exact, the return image matches, and removed-blocker, disabled-target and rejected-reload checks pass.
- The quarry passes at its original, 1 cm forward and return poses: 346,520 / 345,883 / 346,520 covered receivers and 8,547 / 8,603 / 8,547 partial shadows. HDR, count and depth differences are all 0, and the return image is byte-identical to the original.
- All 30 fixture outputs and the 9 quarry HDR/count/depth outputs are byte-identical to the previously verified run.
- Every screen variant without a `diag` prefix has 0 differing visibility pixels against the frozen baseline.

Inspected complete images: `images/quarry-c2-pose-{0,1,2}.png` and `images/fixture-c2-sheet.png`, made with the renderer's Reinhard-plus-sRGB display mapping. They cover outer silhouettes, slab and wedge-rock contacts, rubble, wall crevices, the sharp contact blocker and the soft thin-strip shadow under motion. No compute-specific change is possible: final-binary outputs hash-match the inspected candidate-2 outputs and prior evidence.

## Per-pass GPU time

Release test binary, validation off, headless 1920x1080 at 1x. The workload is 12 quarry draws (13,034,903 triangles), a 0.04 rad disk, 256 rays per covered receiver, exposure -14 and the RLE2 environment. Each process uses the final binary and renders 12 warmup frames. It then renders 48 pipelined frames alternating frozen-baseline and production compute pipelines on the same pose, alternating poses every pair, with no per-frame drain. Cells are medians with [min, max] over 24 frames per side, in milliseconds. The paired figure is the median per-pair ratio.

| Pass | Process 0 base / retained (paired) | Process 1 | Process 2 | Process 3 |
|---|---|---|---|---|
| Receiver raster | 3.250 / 3.355 (1.001) | 4.027 / 4.761 (1.054) | 3.019 / 3.012 (1.001) | 3.579 / 3.561 (1.006) |
| Visibility compute | 19.366 [13.3, 22.4] / 8.853 [7.6, 13.6] (0.470) | 23.372 / 12.190 (0.533) | 17.842 [16.1, 23.7] / 8.081 [8.0, 13.7] (0.479) | 20.601 / 10.399 (0.486) |
| Shading | 3.831 / 3.795 (1.000) | 5.409 / 5.482 (1.025) | 3.486 / 3.486 (0.998) | 4.482 / 4.630 (0.985) |
| Tone mapping | 0.056 / 0.055 | 0.059 / 0.058 | 0.055 / 0.056 | 0.056 / 0.056 |
| Render span | 26.783 / 16.053 (0.614) | 32.991 / 22.570 (0.675) | 24.324 [22.7, 34.1] / 14.647 [14.4, 25.3] (0.618) | 28.873 / 18.525 (0.642) |

Compute-only dispatches in the same processes (10 warmup, 12 timed repetitions): baseline 17.0-21.8 ms, production 8.15-9.91 ms, paired 0.459-0.495. The untouched receiver, shading and tone passes stay at about 1.00 paired, which serves as a control. Absolute levels vary about 20% between processes. Process 1 inflates every pass, consistent with uncontrolled desktop GPU load. The earlier single-sample baseline (25.598 ms span) sits inside the baseline range.

Hardware: Windows/Vulkan, Ryzen 7 7800X3D, RTX 4070 SUPER 12 GB, NVIDIA 617.14. Every GPU process preflighted games, renderlab/selector processes and GPU utilization, and sampled foreign processes throughout. No game ran. The streaming-selector worker's `sel2shadow.exe` deferred three attempts. One screen overlapped a foreign renderlab test binary only during its first 4.5 s of asset loading, before timing. Other user applications, including an animated-wallpaper renderer and other workers' CPU builds, stayed open. This is not an idle-machine certificate.

## Rejected attempts

| Attempt | Result |
|---|---|
| Larger groups: 16x8, 16x16, 32x2 | Exact; 1.08-1.15, slower |
| Swizzled group order | Exact; 0.99-1.01 |
| Per-lane rotation aligning lane directions | Exact; 0.71-0.73 on 8x8, 1.07 relative on 8x4; superseded |
| Row-shaped and 16-lane groups: 32x1, 16x2, 4x4, 8x2 | Exact; no better than 8x4 |
| Other lane splits: `ppx4`, `ppx16`, `ppx32` | Exact; 0.466-0.498 against 0.464 for `ppx8` |
| Unconditional first-hit flag | Exact here, but not when `depth_bias = 0`; replaced by the `tmin > 0` guard |
| Covered-receiver compaction | Analysis only: at most 2.1% lane headroom |
| Early-out for receivers facing away from the light | Analysis only. Double-sided origins already flip toward the light, so there is no geometric back face. Skipping shading `N·L <= 0` would be HDR-exact, but it changes the unchanged count gate and needs normal-map sampling in the receiver raster. Those receivers fall inside the 26,488 fully occluded receivers, which first-hit already makes cheap. |
| AS rebuild or merge to cut BLAS overlap | Analysis only. The 12 placements have distinct yaw and 0.01-scale transforms (world bounds overlap up to 100%), so re-partitioning changes per-triangle intersection coordinates and is not exact. Build flags are already `PREFER_FAST_TRACE` with opaque geometry; compaction changes memory, not visited nodes. |
| Fewer rays, reuse or denoising | Not attempted. The existing gates require exact equality with the 256-ray reference, which any sampling change fails by construction. |

One quarry gate attempt on the final binary aborted at host `memory allocation of 67108864 bytes failed` during asset loading. Free physical memory was 4.95 GiB, with foreign builds running and Explorer holding about 18 GB committed. The unchanged rerun passed after Explorer was replaced by a new process. Both are retained.

## Tests, time and reproduction

The CPU suite passes: 30 `shadow` tests, including the new screen-atom composition check and the dispatch-tile check. The changed-package release `clippy --lib --tests -D warnings`, `cargo fmt --check` and `git diff --check` pass. The 12 other previously ignored GPU suites (hardware, raster, environment and normal) do not use the fresh path. Their evidence is reused, not rerun.

Verification: 12 native GPU processes took 102.19 s successful plus 33.42 s for the aborted one. Recorded build stages took 48.55 s; four further incremental release builds ran inside CPU-test invocations, about 22 s each (one measured at 22.5 s); the cold target warm-up took 108 s. CPU tests took 1.16 s and Clippy 3.7-5.4 s. Waiting for the shared GPU slot took about 25 minutes and is not counted. Implementation editing totals about 22 minutes across six stamped windows from 12:10 to 12:51 UTC, interleaved with CPU work while the GPU was busy. That time is not isolated and must not be derived from wall time.

Evidence: `D:/Coding/renderlab-work/m6-compute-cost/`. It holds `run.py` (preflight, contention sampling, build identity), `builds/<label>/` (copied executables, sources, diffs and SHA-256 identities) and `runs/` (manifests, logs, screens, gate outputs). It also holds `images/`, `scripts/` (CPU coverage, bounds, display and summary tools), `screens-table.txt`, `timing-summary.txt` and `time-accounting.txt`. The final binary is `builds/cand-3/fresh-tests.exe`, SHA-256 `b1e9823aa37103a54a3c13178f4a381b2bd67cb14c86202899f5b22d2efde888`. The screen is `renderer::fresh_shadow_tests::gpu_fresh_compute_cost_screen` with a JSON manifest; variant specs such as `wg8x4+fh` or `ppx8+fh` are composed from text atoms and validated by naga on the CPU.

## Next boundary

Exact scheduling options at this level are largely spent. The two raster passes now total about 6.5-10 ms, and each re-rasterizes all 13 M triangles; shading could reuse the receiver depth, but that change is in `renderer.rs`, outside this unit. A further large compute reduction needs fewer rays per receiver. That is a quality change and needs an independent non-exact quarry reference gate, which does not exist yet.
