# M6 Interactive soft-shadow cost hillclimb, second pass

2026-11-05 local (2026-11-04/05 UTC), based on `7de1ede`, after the [first climb](m6-interactive-climb.md). Scope set by Islam mid-unit: **bit-exact changes only**, no precision or output change, and the shading stage stays general for future user-written shaders. Compact or quantised vertex attributes were dropped unscreened.

**Interactive span 0.857–0.863 → 0.8156–0.8161 ms (−5.2%); hard frame 0.947–0.963 → 0.831–0.847 ms (−12%).** Interactive, Exact and hard outputs are byte-identical to main. **The 2x VSM stretch (≤0.80 ms) misses by 0.016 ms.** No push.

## Result

Complete frames: RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, release test binary, validation off. The registered quarry is 12 full-resident draws (13,034,903 triangles) at 1920×1080 1x with a 0.04 rad disk. `gpu_soft_shadow_complete_paired_timing` runs 24 warmup and 24 paired hard/Exact/Interactive triplets per process. Processes alternated c11 (main), d17, c11, d17, c11, d17. Cells are medians [min, max] in ms, per process.

| Pass | c11 (main) 1 / 2 / 3 | d17 1 | d17 2 | d17 3 |
|---|---|---:|---:|---:|
| Interactive fused trace | 0.4009 / 0.4007 / 0.4023 | 0.4079 [0.4015, 0.9563] | 0.4065 [0.4017, 0.4116] | 0.4064 [0.4025, 0.7641] |
| Interactive shading | 0.3959 / 0.3919 / 0.3967 | 0.3651 [0.3608, 0.8623] | 0.3669 [0.3603, 0.3698] | 0.3671 [0.3609, 0.7673] |
| Interactive tone | 0.0496 / 0.0491 / 0.0496 | 0.0338 [0.0326, 0.2079] | 0.0338 [0.0317, 0.0348] | 0.0328 [0.0317, 0.0358] |
| **Interactive span** | **0.8606 / 0.8567 / 0.8625** | **0.8159 [0.8099, 1.8133]** | **0.8161 [0.8086, 0.8243]** | **0.8156 [0.8067, 1.3643]** |
| Hard trace | 0.3071 / 0.2997 / 0.3071 | 0.2662 [0.2537, 0.2729] | 0.2609 [0.2522, 0.2689] | 0.2625 [0.2543, 0.2686] |
| Hard shading (inline shadow ray) | 0.5963 / 0.5857 / 0.5949 | 0.5312 [0.5220, 1.4660] | 0.5273 [0.5235, 0.5338] | 0.5284 [0.5224, 0.5327] |
| **Hard span** | **0.9628 / 0.9468 / 0.9630** | **0.8470 [0.8262, 1.7879]** | **0.8310 [0.8210, 0.8512]** | **0.8377 [0.8248, 0.8495]** |
| Exact receiver trace | 0.3237 / 0.3168 / 0.3244 | 0.2784 | 0.2777 | 0.2771 |
| Exact shading | 0.3971 / 0.3974 / 0.3994 | 0.3767 | 0.3705 | 0.3690 |
| Exact span (GPU state-bound) | 8.3955 / 8.9873 / 8.4264 | 9.6130 | 9.1634 | 8.3143 |
| Paired Interactive/hard span | 0.8960 / 0.9091 / 0.9048 | 0.9745 | 0.9807 | 0.9777 |

The Exact span is dominated by its 256-ray visibility, which moves between the known fast (7.6 ms) and slow (8.9 ms) GPU states; its trace, shading and tone passes improve as the table shows. Every pass's tone drops 0.015 ms although it reads the same bytes; this was measured, not diagnosed. In complete frames the unchanged fused kernel is 0.004–0.006 ms slower; in the in-process screen it is unchanged (0.3936/0.3932 vs 0.391–0.394).

Against the stored [native Unreal timings](unreal-native-shadow-mask.md) (UE 5.7.4, not relaunched or paired contemporaneously): **1.96x VSM** (1.59650 ms) and **2.63x HRT** (2.14975 ms). The 2x VSM floor is 0.798 ms. This covers the bounded direct-light/shadow subset, not whole-engine parity.

## Retained mechanisms

1. **Tile coverage flags and the scene rectangle in hard and Exact traces.** `primary_trace` and `primary_trace_receivers` trace only pixels inside `scene_pixels`, OR their hits into a workgroup flag and write records (and receivers) only in covered tiles. Exact fresh visibility reads the flag before the record and still writes every count. All frames now shade through one tile-aware `primary_shade_tiles` per module. Kernel screens (in-process, paired; production vs the c11 kernel): hard trace 0.2285 vs 0.2950, hard shading 0.5636 vs 0.5914, Exact trace 0.2445 vs 0.3126, Exact shading 0.4104 vs 0.4405 ms.
2. **Shading dispatches only the scene region.** The region is the rectangle widened to whole 8×4 trace tiles. The same dispatch appends rows of 16×16 clear blocks, each lane storing one column, that write the clear colour everywhere outside the region with the same compute store. Launching about 107k 16-lane groups over background tiles had cost more than their stores. Screen g1 (d3 binary): region shading alone 0.343 vs 0.398 ms; region shading plus a separate 16×16 clear pass 0.7572–0.7598 vs 0.8018 for the fused+shade pair. The region is CPU-computed and read from a 16-byte storage buffer (see the hazard below).

## Determinism hazard (found and avoided)

The first region-dispatch builds (d5–d15) failed the static-repeat check in 25–60% of acceptance runs, always at the same frame of the sequence. Two consecutive identical Interactive frames differed in 22,361 pixels by one f16 ulp. Dumps showed:

- hit-record barycentrics moved by about 1e-5 at 178,120 pixels across all 12 placements;
- the GPU frame uniform equalled the uploaded bytes;
- TLAS and BLAS build counts were unchanged;
- a **freshly created pipeline of the same seed reproduced the first frame exactly**, while the long-lived pipeline object kept the deviant output.

In one d15 failure the long-lived *shading* pipeline deviated instead, with identical hits. So the driver changed a pipeline's compiled arithmetic mid-run. The trigger was a dispatch layout derived from frame-uniform values: region offsets computed from `primary_frame.scene`. It was not the clear itself (clear first, separate pass and in-dispatch variants all failed) and not the kernel text (c11's fused kernel also deviated in d14).

Reading the region from a storage buffer removed it: **16 of 16 acceptance runs pass on d16/d17**, against 0 failures in 10 runs of c11/d3. The mechanism is inferred, not confirmed; there are no driver counters. Any kernel whose control flow or addressing depends on uniform data can lose bit-exactness on this driver, and naga emits no `NoContraction`. Static-repeat checks should run several times per candidate.

## Screens and rejected attempts

All screens are in-process and paired, with 96 warmup and 48 timed rotations per process, 2 processes each. Medians in ms; r is the paired ratio.

| Attempt | Result | Status |
|---|---|---|
| Diagnostic decomposition of shading (e2/e3) | Fixed cost (record, store, clears) 0.082; +3 index loads 0.163; +9 vertex loads 0.372; full 0.396. Lit body, textures and environment add only ~0.014 | Limiter: dependent gather chain plus background group launches |
| Skip the vertex-colour load (every quarry colour is 1) | r 0.974–0.989 on c11 shading; **r 1.020–1.022 on d17 region shading** | Rejected; would also need a `Mesh` field built in streaming code |
| Publish hit triangles' vertex indices from the traces | Shading −0.010, fused +0.010; pair 0.7956/0.7982 vs 0.7926/0.7946 | Rejected; also a 33 MB target |
| Skip tile-flag loads outside the rectangle | r 1.013–1.019 | Rejected |
| Two pixels per shading thread | r 1.085–1.090 | Rejected |
| Soft module without the single-sided candidate loop | r 1.003–1.006 | Rejected |
| Diagnostic: shading gather issued in the fused pass | +0.115 ms | Gather does not hide behind ray traversal |
| Region dispatch for the fused pass and traces | ≤0.008 in sequence (g1); triggered the hazard with a uniform-derived region | Not retained; untested with a storage region |
| Separate 8×8 or 16×16 clear pass (g1) | 0.0174 / 0.0113 ms per pass | Superseded by in-dispatch clear blocks |
| Compact or quantised vertex attributes | Dropped by the bit-exact scope | Not screened |

## Equivalence and stability (d17, validation on)

- Traced Interactive acceptance: 36 of 36 files identical to `accept-c11` (main), including static repeat, return, reload and raster fallback. Eight further repeats pass, as did eight on d16.
- Primary traced acceptance: 285 of 285 identical to `primary-accept-c9`. Quarry mask replay: 66 of 66. Thin and stress fixture replays: 575 of 575 each. The single-sided/mirrored fixture passes.
- The traced lit suites pass 11 of 18, the same 7 failing as before (traced renderers cannot run them).
- Because every output is byte-identical, the Level-B gate (U1–U10, S1–S5, G7-B) and the primary-ray acceptance error carry over unchanged.

## Generality constraints

- **Existing, unchanged here:**
  - Traced shading rewrites the user's `fs_lit` by exact text seams (`compute_lit`): the signature, the three `textureSample` calls, the `discard` form, and the geometric, tangent and present seams. It rejects implicit derivatives and other bound textures, and binds a fixed three-texture material per draw slot. A differently styled user shader fails traced shading until a declared shading-stage interface replaces the seams.
  - Soft visibility is computed for `lights[0]` only, and every shadowed light reads it.
- **Extended here:** misses are a constant clear colour. The tile flags (first climb) and the region clear blocks store it; a programmable background or miss shader needs a miss-shading invocation there instead. The clear-block branch of `primary_shade_tiles` is the single place to change. The region decode assumes 4×4 shading groups.
- **Not added:** nothing lighting-model-specific entered the hit record or the shading dispatch. The record is still placement, primitive and barycentrics.

## Interface changes

- Every traced frame computes `scene_pixels` on the CPU (O(placements) per frame). `PrimaryState` keeps the rectangle and the tile-aligned region, which is uploaded as a 16-byte storage buffer (shading binding 22) only when it changes.
- `Shading` is `Hard | Soft`; `primary_shade` was removed. Trace layouts gain the tile-flag output (binding 20); shading and fresh-visibility layouts read the flags (binding 21).
- Hard and Exact hit records and receivers are valid only in covered tiles; tests read them through `hit_record`, which fills uncovered tiles with misses.

## Checks and time

- CPU filters `shadow primary soft` pass (36 + 7 + 3; GPU tests ignored).
- Release Clippy `--lib --tests -D warnings`, `cargo fmt --check` and `git diff --check` pass.

**Implementation and analysis:** about 22:15–00:30 UTC (2.25 h elapsed), interleaved with verification and the hazard bisection; editing time was not isolated.

**Verification:**

- 17 release test builds took 615.8 s (d1 cold 109.7 s; the others 24–38 s); Clippy took 24.3 s.
- 152 offscreen GPU child processes took 1,694.2 s; 48 exited nonzero. These were the hazard bisection (mostly deliberate repeats), one storage-texture limit failure (d1), the d4 fresh-layout binding error and the 7 known suite cases.
- The shared GPU slot was held in 206 windows, 2,590.8 s in total, at most 111.1 s each. All were released; the last before this record at 00:26:18 UTC.
- 63 runs deferred: 29 while the Unreal timing worker held the slot, 34 on free RAM below 8 GiB (minimum 4.29 GiB). All 161 accepted preflights had at least 8.10 GiB and no game, Unreal or foreign renderlab GPU job. Up to 20 foreign cargo/rustc processes ran during some timed runs; all samples are retained.

Evidence is in `D:/Coding/renderlab-work/m6-interactive-climb2/`:

- `run.py`, `build.py`, `clippy.py` and `retry.py`;
- `builds/d1`–`d17`: binaries, SHA-256, HEAD and source diffs. d17 is `2d5061fe…44d6`;
- `runs/`: manifests, logs, screens, timings, repeat and compare results, and the failure dumps;
- `scripts/`: variant generators, `verify.py`, `repeat_accept.py`, `timing_runs.py` and the dump analysers;
- `gpu-windows.json`.

## Next boundary

0.016 ms remains to 2x VSM. The exact levers left are small and uncertain:

- region dispatch of the fused pass with the storage region, ≤0.008;
- the complete-frame fused-pass slowdown, 0.004–0.006.

Larger cuts need a cheaper closest-hit traversal or a shorter dependent gather chain. Both would need data-layout changes, which this scope excludes.
