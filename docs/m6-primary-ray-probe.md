# M6 primary visibility: probe passes, integration rejected

Superseded for integration by the [production path](m6-primary-ray-production.md): the prototype's budget miss was recorded as a miss, and the retained path passes quality acceptance.

2026-11-04, based on `d8e3c38`. The [first-unit](m6-path-to-2x.md#4-first-unit-traced-primary-visibility) closest-hit probe passes its 0.75 ms stop gate: **0.294 ms**, versus **3.481 ms** for the current receiver raster. Every enumerated receiver difference is explained. The subsequent deferred-shading prototype misses the integration budgets, including **11.085 ms soft** and **2.096 ms hard** GPU spans. It is rejected and removed from the working implementation. This is not M6 acceptance or a performance comparison with Unreal.

Only the isolated test probe, its shaders, and this record are retained. Production renderer and fresh-shadow implementation files match the base. Streaming and viewer files were not changed. The failed prototype remains available as a separate local patch and frozen source/binary snapshots.

## Workload and method

- Windows, AMD Ryzen 7 7800X3D, NVIDIA RTX 4070 SUPER, driver 617.14, Vulkan. Release library-test binaries; experimental ray-query acknowledgement enabled.
- Full-resident quarry, 12 draws/instances, 13,034,903 triangles; 1920x1080, 1x, opaque double-sided, original 4K textures, 16x anisotropy, exposure -14, unchanged RLE2 environment and 0.04-radian source radius. No LOD or quality reduction.
- Camera sequence: original, registered 1 cm X translation, return to original. These are the three sequence entries used by the fresh-compute evidence, not three distinct viewpoints.
- Timing follows the [fresh-screen paired method](m6-fresh-compute-cost.md): one process, 12 warmup frames, 24 raster/candidate pairs, identical pose within a pair, alternating poses between pairs, unique timestamp slots and one final readback. Validation is disabled for timings and active in separate image/readback processes. No per-frame drain or acceleration-structure rebuild is timed.
- All GPU tests were offscreen. Each acquired the atomic shared GPU directory before preflight and released it afterward. Fourteen recorded native measurement windows lasted at most 17.55 seconds; minimum free RAM was 8.28 GiB. Preflight found no game, Unreal, foreign Renderlab job or heavy build. No stale lock was removed. A separate 38-second slot covered the offscreen report-chart preview; its release is recorded too.
- Ordinary desktop activity remained: sampled GPU utilization was nonzero before tests. These are paired results under desktop contention, not an idle-machine certificate. The spreads below include spikes; no outliers were discarded.

The initial timing attempt used a different ordering that confounded pose and cache state. It is preserved as superseded evidence. The final Phase A result below uses the documented alternating-pair method; `timing-method-correction.txt` records the correction.

## Phase A: closest-hit probe

The 8x4 compute group unprojects near (reversed-Z 1) and far (0) through the inverse of the same f32 view-projection matrix. It traces the full segment with forced opaque geometry and no terminate-on-first-hit flag, using the existing shadow TLAS/BLASes. It writes instance+1, primitive, full-f32 barycentrics and projected depth. The timing path writes 20 B/pixel: RGBA32U hit plus R32F depth, 41,472,000 bytes total. An additional 16 B/pixel world-position output is enabled only for validation, through a pipeline override.

| Pass | Median ms | Min ms | Max ms | Samples |
| --- | ---: | ---: | ---: | ---: |
| Current receiver raster | 3.481088 | 2.903008 | 3.795808 | 24 |
| Closest-hit primary probe | 0.293840 | 0.280064 | 0.806880 | 24 |

Median per-pair trace/raster ratio: 0.083823. The stop gate is on the median; the single maximum above 0.75 ms is included. The median is far enough below the gate that a second timing process was not needed.

### Complete mismatch enumeration

The diagnostic raster adds IDs but uses the production receiver's vertex arithmetic, interpolation, strict reversed-Z Greater test and draw order. Its world/depth output is byte-identical to the production receiver at every pixel in all three captures. The comparison enumerates the union of coverage differences, winning instance/primitive differences and world-position differences greater than 0.1 mm. The counts overlap, so the first three difference columns do not sum to the union.

| Sequence entry | Raster covered | Trace covered | Coverage XOR | Different winner, both covered | World >0.1 mm, both covered | Enumerated union |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Original | 346,520 | 346,517 | 3 | 2,091 | 570 | 2,545 |
| 1 cm translation | 345,883 | 345,882 | 3 | 2,041 | 548 | 2,496 |
| Return | 346,520 | 346,517 | 3 | 2,091 | 570 | 2,545 |

| Explanation | Original | Translation | Return |
| --- | ---: | ---: | ---: |
| Silhouette or shared source edge | 2,092 | 2,042 | 2,092 |
| Raster subpixel receiver interpolation | 451 | 452 | 451 |
| Reversed-depth first-winner tie | 1 | 1 | 1 |
| Raster subpixel depth ordering | 1 | 1 | 1 |
| Unexplained | **0** | **0** | **0** |

Silhouette/shared-edge cases lie within 0.002325 pixels of an actual projected source-triangle edge, inside the measured raster's 8-bit subpixel snapping bound. Hardware intersection uses the continuous triangle, while raster coverage uses snapped edges. The translated view loses two covered pixels and gains one; the original loses three. Large world/depth changes at these pixels select a different nearby projected surface: the largest world displacement is 0.521 m, not a submillimetre numerical error.

The same-triangle cases reconstruct onto the source plane within 1.042 micrometres and reproject within 0.001947 pixels of the raster sample. A subpixel screen-space interpolation shift on a grazing microtriangle produces world shifts over 0.1 mm (up to about 2.2 mm). Independent f64 source-plane and projection checks, together with snapped-screen barycentric reconstruction, explain these cases. The classifier uses a 2-micrometre plane bound and half of a 1/256-pixel subpixel bound plus f32 allowance; it does not classify an arbitrary nearby world point as a match.

Four non-edge winning-triangle differences were checked by drawing each competing triangle separately and reading its actual GPU f32 depth. At original pixel (591,617) both depths have bits 973644537; at translated (852,707), both have bits 975610236. Strict Greater therefore keeps the first submitted triangle. At original (844,622), raster/alternative depths have bits 973831696/973831674; at translated (1065,514), 973316554/973316531. Raster subpixel depth interpolation reverses the continuous-ray ordering in these two cases. This is measured isolated-triangle evidence, not an assumed epsilon tie.

Depth is not byte-identical. Among pixels with the same winner, median absolute depth difference is 2.91e-10 (5 ULP); p99 is 3.03e-9 (66/67 ULP), and the maximum is 2.001e-7 original / 1.084e-7 translated. Raster-interpolated depth and reprojection of a ray-computed world point use different f32 arithmetic; grazing subpixel interpolation accounts for the larger same-surface tail. Across all jointly covered pixels, including different surfaces at edges, maximum absolute depth difference is 3.714e-5 / 3.860e-5. Complete per-pixel mismatches, plane/projection residuals and depths are in the CSVs, with aggregate depth distributions in `depth-summary.json`.

All traced covered pixels have valid barycentrics. Reconstructing their world position from the hit triangle and barycentrics differs from the stored ray point by at most 2.809 micrometres; none exceeds 0.1 mm. Original and return hit, world, depth and raster-ID buffers are byte-identical. Both paths use 1x pixel centres: there is no MSAA sample-location discrepancy, and this does not qualify a 4x path.

## Phase B: bounded trial and rejection

The test-only prototype reused existing vertex/index/tangent buffers with storage usage, without a second geometry copy or any AS change. A 16 B/pixel hit record held global triangle ID, full-f32 barycentrics and depth. Per-material compact worklists added 4 B/pixel worst-case plus small batch metadata. Exact triangle normals generated receiver origins. Existing lighting, material, normal-map and environment WGSL supplied shading; soft shadows retained the exact 256 directions/seed/estimator. No history, ray-count reduction or denoising was added.

Texture LOD used analytic ray differentials. The prototype differentiated near/far unprojection, intersected each differential with the hit triangle's plane, then mapped the resulting world-position derivatives through the triangle's affine UV basis. Material samples used `textureSampleGrad` with the existing 16x anisotropic sampler and LOD clamps; the DFG lookup used explicit level 0. This avoids inventing a fixed mip. It intentionally differs from raster quad derivatives, so raster equality is not its quality oracle. Shader composition had CPU validation, but transactional reload on the new path was not qualified.

The first full-image-per-material trial cost 13.54 ms soft. Compact worklists reduced it to 10.30 ms; a subsequent 32-thread reconstruction/shading trial measured 10.25 ms. A final shared-gradient and position-only reconstruction variant did not resolve the budgets. No additional optimization direction is claimed to work.

Final same-binary results, medians [min, max], n=24 per side. Complete spans are timestamped spans, not sums of independently computed pass medians.

| Pass | Soft prototype ms | Hard prototype ms | Integration target |
| --- | ---: | ---: | ---: |
| Trace only | 0.293648 [0.279200, 0.324288] | 0.293840 [0.279936, 1.283008] | <=0.4 ms primary |
| Worklist count / prefix / scatter | 0.026144 / 0.003696 / 0.017040 | 0.026352 / 0.003872 / 0.017056 | Accounted in span |
| Receiver reconstruction | 0.315872 [0.309472, 0.912128] | 0.312928 [0.309536, 0.964320] | Part of usable primary visibility |
| Complete receiver span | 0.792416 [0.663200, 1.287584] | 0.679840 [0.661888, 1.832896] | <=0.4 ms: **miss** |
| Deferred shading | 0.564496 [0.531776, 1.341888] | 0.538912 [0.530240, 1.258368] | <=0.4 ms: **miss** |
| Shadow visibility | 9.582032 [8.465632, 13.779744] | 0.412080 [0.409728, 0.811840] | Unchanged exact soft estimator |
| Tone | 0.055280 [0.051200, 0.591744] | 0.056320 [0.051200, 0.561152] | Unchanged |
| Complete GPU span | **11.084656 [9.984000, 16.159520]** | **2.095920 [1.697056, 3.198912]** | <=9 / <=1 ms: **miss** |
| Paired raster complete span | 16.670720 [15.301632, 25.630720] | 5.015904 [4.567040, 8.136544] | Comparison only |

The raw trace meets 0.4 ms, but producing usable receivers costs more. Even the best observed reconstruction-plus-trace and shading costs miss their budgets; both complete-span minima miss too. Desktop spikes do not explain away the rejection. The test-only trial has been removed and saved as `rejected-primary-integration.patch` against the retained probe checkpoint `437b0174ea0701436aea2cc0ef02d2bf5971ba97`.

## Verification and limits

- Retained source: focused release CPU filter `shadow`: **31 passed, 17 GPU tests ignored**, zero failures; shader test compiles both probe shaders and both world-output override variants to SPIR-V. Release Clippy for `renderlab --lib --tests -- -D warnings`, formatting and diff checks pass. Final rebuilt retained binary is in `builds/retained-final/`.
- Probe readback and final soft/hard prototype readbacks pass with active Khronos validation, zero errors and no missing layer. Timing runs have validation disabled. Prototype CPU tests passed (32), and prototype Clippy passed before removal. Earlier override-resolution test failure, a test call-site compile failure and three Clippy needless-update findings were corrected; failed attempts remain in the time/evidence ledger.
- Inspected full 1920x1080 Phase A raster-depth, trace-depth and mismatch maps at all three sequence entries. Inspected final soft and hard prototype images at original, translated and return entries, raster original comparison, +4-stop original visualizations, and the soft original absolute-difference image amplified 32x. Inspected outer silhouettes, slab/wedge contacts, rubble and crevices; no gross missing geometry, detached shadow or stale return image was apparent. Small pixel changes remain; inspection is not a quality-gate pass.
- Full-image soft HDR difference from production raster: mean absolute 1.901e-5 / 1.878e-5 / 1.901e-5, RMS 5.396e-4 / 6.415e-4 / 5.396e-4, maximum 0.3870 / 0.6681 / 0.3870. These include changed receivers, exact normals and texture gradients; they do not isolate texture LOD error or measure error against ground truth.
- Prototype return HDR, hit, barycentric, depth and shadow-count buffers are byte-identical to the original for both soft and hard runs. The hard run leaves different world/normal payloads at 469 background pixels that were covered only in the intervening view. These payloads are deliberately unspecified when hit ID is zero; the shadow kernel checks hit coverage first, and all 469 have background count 256. Covered receiver payloads match exactly. `prototype-return-equality.json` and `prototype-background-return.json` preserve this distinction.
- **Not run after cost rejection:** regenerated 16,384-ray references on the new receivers; Level A G1-G7; supersampled quarry error comparison; new-path mirrored/nonuniform normal-map, environment, material-assignment and transactional reload acceptance. Therefore exact-shadow quality, no-worse reference error and the first unit as a whole are **not accepted**. There is no production shading integration to ship.

## Evidence and time

All private captures, generated measurements, helper scripts and binaries are outside Git under `D:/Coding/renderlab-work/m6-primary-ray/`.

- `runs/probe-validation-final/`: raw buffers, cameras, every mismatch CSV, isolated GPU depths and complete diagnostic PNGs; `enumeration.json`, `hit-reconstruction.json`, `depth-summary.json` and equality records summarize them.
- `runs/probe-timing-final/`, `paired-summary-final.json`: final Phase A paired samples. Binary `builds/probe-7/primary-tests.exe`, SHA-256 `b4f5f773a62cd9bd2d2b4f3f57f501cda95d357ef7c97de6876942473c08fce6`; source snapshot and identity adjacent.
- `runs/integration-{soft-final-timing,hard-timing}/summary.json`: final trial timings. `runs/integration-{validation-final,hard-validation}/`: complete raw HDR/count/hit/depth captures and display PNGs. Binary `builds/integration-final-2/primary-tests.exe`, SHA-256 `ae74059338155585bbabf8c9bdfc3ec11ea0069a3359099ee982b5925d8b3b4e`; source snapshot adjacent.
- `run.py`, `classify.py`, `display_integration.py`, manifests and `input-identities-final.json`: reproduction settings and private-input hashes. `gpu-coordination-final.json`, per-run preflight/acquire/release records, and `chart-slot-released.json`: GPU coordination evidence. No GPU slot remains held at handoff.
- `rejected-primary-integration.patch`: 75,323 bytes, SHA-256 `53414a82a6167f3daeb137d6c8cb2a9dc1244b3dc9a91296f316b7f0731a4a38`; applies to the retained probe checkpoint. It is evidence of a rejected experiment, not an accepted implementation.

**Verification time:** instrumented native jobs total **602.06 seconds (10m02s)** across 49 successful/failed attempts: optimized builds 438.24 s, Clippy 47.97 s, native GPU test processes including asset load/readback 112.36 s, CPU test processes 3.50 s. This includes superseded variants and failures. The initial dev `cargo check` was a further 34.99 s; exhaustive enumeration and hit reconstruction were 3.43 s and 3.81 s. Small manual CPU checks, image inspection, analysis and report/preview time were not individually stopwatched, so 10m02s is not an all-inclusive verification total. The job ledger is `verification-time-final.json`.

**Implementation time:** design/implementation began at 15:46:12 UTC and continued through the final trial before 18:09 UTC, interleaved with those checks and inspection. Net editing time was not isolated or stopwatched. Approximately 31 minutes (16:11:53 to about 16:43 UTC) awaited the Unreal worker's GPU slot while CPU work continued; early RAM deferrals also overlapped CPU work. These wall-clock intervals must not be reported as net implementation or GPU execution time. Documentation and packaging followed separately. No window, viewer, game capture or push was performed.
