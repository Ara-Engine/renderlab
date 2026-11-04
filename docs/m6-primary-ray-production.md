# M6 traced primary visibility: production path

2026-11-04. Traced primary visibility is now a selectable production path. It follows the [probe](m6-primary-ray-probe.md) and replaces the rejected test-only prototype. It passes quality acceptance and is retained. After the hillclimb, the receiver target is met; the shading target is met only in some processes, which is recorded as a miss. Raster stays the default and the fallback. This is not a matched-Unreal or 2x result.

## What changed

- **API.** `Renderer::set_primary_visibility(PrimaryVisibility::Traced | Raster)` selects the path; `primary_visibility()` reads it back. `FrameReport::primary` reports the path each frame actually used. Selecting `Traced` compiles the traced modules from the installed source, transactionally. While traced is selected, every reload recompiles them in the same transaction.
- **Requirements.** Traced visibility needs a hardware-shadow renderer at 1x whose device enables `TRACED_FEATURES` (binding arrays with non-uniform indexing, plus hit-vertex return) and `traced_limits`. Every acceleration structure such a renderer builds allows vertex return.
- **Where it applies.** Only lit views trace. Diagnostic views and virtual geometry rasterize. A frame may have at most `TRACED_DRAWS` (64) draws. When hardware shadows are enabled, materials must be double-sided, as before. Without shadows, single-sided draws are supported: the trace culls their source-back faces.
- **Frame structure.** Each stage is one dispatch:
  1. Trace (8x4 groups): a closest-hit camera ray per pixel centre against the existing TLAS. It writes a 16 B hit record (placement+1, primitive, f32 barycentrics). Soft frames also write the exact receivers at hits: the ray point and depth, plus the plane normal from the hit triangle's returned positions.
  2. Fresh visibility (soft frames only): the unchanged 256-ray estimator. Coverage comes from the hit record, so miss pixels never write receivers.
  3. Deferred shading (4x4 groups): the unchanged `fs_lit` body, made into a compute function. It uses analytic ray-differential texture gradients and descriptor-indexed geometry and materials. In hard frames, `hard_shadow` traces inline, as the hardware raster path does.
  4. Tone mapping, unchanged.

  There is no compaction, no per-draw dispatch and no second geometry copy. Instance transforms are packed by placement, about 128 B each.

Code: `src/primary_ray.rs`, `src/primary_ray_gpu.rs`, `shaders/primary_rays.wgsl` and `shaders/primary_trace.wgsl`, plus integration in `renderer.rs`, `fresh_shadow_gpu.rs`, `lighting_gpu.rs` and `hardware_shadow_gpu.rs`. The exact 256-ray soft-shadow semantics are unchanged.

## Acceptance (final source, build b12; outputs byte-identical to the first accepted build)

Validation was active with zero errors on every acceptance run. The workload is the full-resident quarry: 12 draws, 13,034,903 triangles, 1920x1080, 1x, 16x anisotropy, RLE2 environment and 0.04 rad disk. The sequence is original, +1 cm and return.

- **Level A G1-G7: pass.** The 16,384-ray reference was regenerated on the traced receivers (seeds 1-64, spread seeds 65-80). The gate script is `support/fresh_gate.py` from `e1c7822`, unchanged (sha256 `3d2207…a810`). There were 0 failures across 3 poses × 4 seeds and both motion pairs; the q floors were 0.0182, 0.0180 and 0.0182. The gated candidate is the production estimator itself. Its masks are synthesized only so that the gate's replay identity holds, and that identity is asserted. The G7 images were inspected: production minus reference shows estimator noise along penumbrae only.
- **Receivers against the earlier raster-receiver reference** (same estimator and seeds):
  - Same-surface pixels: mean difference −3e-5, p99 4.3e-4.
  - 258 and 262 pixels per pose (0.075%) exceed 5 standard errors. Of these, 71 and 58 are adjacent-triangle winners at shared edges. The rest are partial receivers whose surface point moved by a median 12 µm (maximum 0.57 mm) on the same plane. Their differences are about 10× what neighbouring-pixel penumbra gradients predict, which is consistent with sub-pixel micro-occlusion on rough scans. They are not concentrated on scan seams. The traced receiver is the exact pixel-centre surface point.
- **Supersampled quarry error** (scan-results metric, 12 registered frames against `reference-4-aniso16`): pass. The traced and raster images are base-colour outputs through the same HDR route.

  | Path | Edge MAE | Texture MAE | Smooth interior MAE |
  |---|---:|---:|---:|
  | Traced | 0.0076817 | 0.0109507 | 0.0029186 |
  | Raster | 0.0076782 | 0.0109546 | 0.0029198 |

  The rule is MAE ≤ baseline × 1.05 + 0.001 for each frame, region and motion pair. That route differs from raster `fs` by at most 1 code.
- **Full HDR against raster production.** Soft: mean absolute difference 1.90e-5 and 1.88e-5, RMS 5.40e-4 and 6.42e-4, maximum 0.387 and 0.668 (original and translated poses). Hard is within 3e-7 of those numbers. Coverage is 346,517 traced against 346,520 raster pixels. Repeat and return outputs are byte-identical.
- **Reload on the traced path.** Invalid WGSL and lit-seam drift are rejected and the image is retained. A tint edit is applied and its revert is byte-identical.
- **Single-sided fixture** (traced against raster): front faces visible, back faces culled, mirrored instances keep their source front, double-sided back faces visible. 3 of 24,576 pixels differ, all on coverage edges.
- **Existing lit suites, raster mode:** 18 of 18 pass. **Traced mode** (`RENDERLAB_TEST_PRIMARY=traced`): 11 of 18 pass, including lighting photometry, environment constant photometry, the mirrored nonuniform normal-map frame oracle, normal-map replacement and mixed draws, the geometric shadow plane, environment ownership and reload, and the material suites. These 7 cannot run on a traced renderer as written:
  - four environment probes reload an `fs_lit` that has no composition seams, which hardware renderers already reject;
  - two use virtual geometry under hardware shadows, which is rejected;
  - one asserts that traced regular and raster virtual images are bit-identical.
- **Images inspected:**
  - full traced soft (registered and +4 stops), raster soft and traced hard;
  - ×32 HDR differences, visibility, traced-minus-raster visibility ×8 and new-minus-old reference ×8;
  - 2× crops of contacts, rubble, crevices, silhouettes and the wedge;
  - G7 sheets.

  Silhouettes, contacts, crevices and rubble are complete. The differences are sparse, unstructured points.

## Per-pass GPU time

Release test binary b11 (identical to b12 except for rustfmt), validation off, paired in-process against raster with pipelined frames. Medians [min, max] in ms.

| Pass | Soft, 3 processes, 24 pairs | Hard, 2 processes, 48 pairs after 150 warm pairs | Target |
|---|---|---|---:|
| Traced receivers / trace | 0.315 [0.302, 0.335], 0.315, 0.321 | 0.306 [0.292, 0.314], 0.306 | ≤0.4 |
| Raster receiver raster | 3.624, 3.255, 2.852 | — | |
| Fresh visibility (traced / raster) | 10.004 / 10.041, 8.829 / 8.753, 7.602 / 7.608 | — | |
| Traced shading | 0.416 [0.396, 1.127], 0.405, 0.398 | 0.597 [0.581, 0.608], 0.596 (inline shadow rays) | ≤0.4 |
| Raster shading pass | 4.103, 3.845, 3.278 | 4.244, 4.240 | |
| Span (traced / raster) | 10.927 / 17.987, 9.625 / 16.028, 8.388 / 13.812 | 0.964 / 4.308, 0.961 / 4.306 | soft ≤9, hard ≤1 |
| Paired span ratio | 0.601, 0.593, 0.607 | 0.223, 0.223 | |

Processes fall into a fast or a slow GPU state that moves every pass by about 25%: fresh visibility is 7.6 ms in the fast state against about 9.5–10 in the slow one. Short hard frames also let clocks drop without a long warmup. Earlier hard runs without warmup are kept but not used.

**Misses.**

- Soft shading meets 0.4 ms only in the fast state (0.398); the slow-state processes measured 0.405 and 0.416.
- The soft span meets 9 ms only in the fast state (8.39). The unchanged 256-ray estimator, at 7.6–10 ms, dominates.
- Hard shading, which includes the shadow rays, is 0.60 ms; the hard span of 0.96 ms meets its target.

## Hillclimb, one mechanism at a time

Each step was compared in paired processes against its predecessor. Non-diagnostic changes reproduce every acceptance output byte for byte.

| Step | Receivers (ms) | Shading (ms) | Limiter named | Evidence |
|---|---:|---:|---|---|
| Prototype (prior session) | 0.792 | 0.564 | — | rejected patch |
| v1: production port, per-draw compaction | 0.756–0.758 | 0.539–0.541 | 12 serialized indirect dispatches per stage: about 8 µs (receivers) and 10 µs (shading) fixed per draw, measured per batch (731 px: 3.7 µs; 137k px: 69 µs), plus compaction passes | b2/b3 |
| A: descriptor-indexed single dispatches, no compaction | 0.664 | 0.448 | hard frame: 1.09 → 0.95 ms | b4 |
| B: 4x4 shading groups | — | 0.400 (screen ratio 0.814; hard 0.992) | Shading is gather-bound: replacing the lit body changes nothing (ratio 1.04); textures 0.898, environment 0.861; sensitive to per-SM cache footprint | b5 screen, b6 |
| C: hit-vertex return for receiver normals | 0.423–0.434 | — | Post-hit vertex gather: removing it gives ratio 0.650. The vertex-return flag leaves fresh visibility (7.640 ms fast state) and AS bytes (911,788,544) unchanged | b7, 5 alternating pairs |
| D: coverage from the hit record, no miss receiver writes | 0.315–0.317 | — | Miss-pixel receiver writes, 32 B × about 1.73 M pixels (ratio 0.838) | b8, 3 alternating pairs |

Not retained:

- Trace workgroup shapes: 8x8, 16x8, 4x8, 4x4 and 8x2 were all 1.04–1.10.
- Shading shapes 16x8, 2x4 and 2x2 were slower; 4x8 gave 0.858.

Remaining shading ideas, not attempted: cheaper miss handling (tile occupancy or a split hit record, about 5% estimated from a 0.945 diagnostic) and attribute compression, which changes data precision.

## Limits and memory

- Hit records take 16 B per pixel (33.2 MB at 1080p).
- Fresh receivers are unchanged at 36 B per pixel (soft).
- The raster depth target is still allocated in traced frames.
- Draw tables are rebuilt only when the draw sequence changes.
- Traced frames require ray-query hardware and the experimental wgpu features named above. Without them, `set_primary_visibility(Traced)` errors and raster is used.

## Evidence and time

Evidence is in `D:/Coding/renderlab-work/m6-primary-prod/`:

- `run.py`, which takes the atomic slot lock and preflights for games, Unreal, foreign GPU jobs and at least 8 GiB of free RAM;
- `builds/b1`–`b12`, with source snapshots; b12 is `fbb9071f…1653`;
- `runs/` (acceptance, quality, timing, kernel screens and suites);
- `gate/`, which holds the unchanged e1c7822 scripts with hashes;
- `scripts/` and `gpu-windows.json`.

Verification time:

- GPU: 131 offscreen child processes took 1,107.8 s, across 140 slot windows. Each window was held for at most 82.7 s; together they were held for 1,270.7 s, and all were released, the last at 20:06:09 UTC.
- Deferrals: 8 runs deferred on RAM below 8 GiB (minimum 6.5 GiB); every accepted preflight had at least 8.77 GiB free.
- CPU: the gate took 106 s and the quality scoring 20 s. About fourteen release test builds took 27–41 s each; the first was 88 s cold. Clippy took 27 s, the workspace check 24 s and the CPU tests 32 s.

Implementation ran from 18:05 to 20:07 UTC, interleaved with these checks; editing time was not isolated.

There was no window, viewer, game capture or push.
