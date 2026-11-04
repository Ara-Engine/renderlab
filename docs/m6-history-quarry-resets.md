# M6 quarry history: bounded reset diagnosis

**Nearest-anchor geometric incompatibility dominates the executed moving work.** The unchanged normal/plane checks cause surface resets on 71.72% / 71.68% of final covered receivers. Those resets account for 98.28% / 98.33% of actual diagnostic queries. This explains why disabling narrow resets cannot remove most of the work observed in [the cost run](m6-history-quarry-cost.md). It does not attribute production milliseconds to individual branches, establish a four-ray cost, prove warp divergence, or identify a guide implementation bug.

One release test build and one headless native session rendered exactly four 1920×1080, 1x frames: cold pose 0, registered 1 cm pose 1, reverse pose 0, exact-static pose 0. No policy, tolerance, quality, baseline or gate changed. The accepted hard-shadow source is restored; nothing was committed or pushed. The only retained checkout addition is this report.

## Inputs and CPU exclusions

Base `65d0f2d82f3b579f3fd25be18557c228a4fc7fa0`. The measured cost candidate was reconstructed from its exact `candidate.patch`; 109 of its 111 recorded inputs remained byte-identical. Only `src/shadow_history.rs` received cfg(test) instrumentation, `src/renderer.rs` a cfg(test) declaration, and one private quarry test was added. A private WGSL override observes the original predicates without changing them. The cost feature/default control, radius 0.04, biases, strict main-depth Greater, visibility prepass and draw order are unchanged.

All five cost-run content identities were rehashed successfully. Paths come from that run's `content-identities.json`: original full quarry, limits, `m6-hardware-msaa/quarry-cameras.json`, exposure -14 lighting and `sun128.rle2` with default environment parameters. The sparse test's different lighting was not reused. Each frame submits twelve draws and 13,034,903 triangles, full resident geometry, original texture chains and 16x anisotropy. Assets were read in place, never copied.

CPU reset reasons are **1, 0, 0, 0**; invalidation-box counts are **0, 0, 0, 0**. Settings, lights, revision (`Revision(1)`), extent and all twelve placement identities remain unchanged. Moving matrix maximum delta is **0.009616248309612274**, below the 0.25 cut threshold; the reverse has the same absolute delta. Static matrix/view delta is exactly zero and the cache branch is selected. Full matrices and comparisons are retained in `cpu-input.json` and `cpu-state-*.json`.

## Actual work

Full-image denominator: **2,073,600 pixels**. Counts include invocations which eventually lose depth testing.

| Frame | Final covered | Actual queries | Queries / covered | Queries / full image |
|---|---:|---:|---:|---:|
| Cold pose 0 | 346,520 | 88,710,656 | 256.0044 | 42.7810 |
| 1 cm pose 1 | 345,883 | 64,623,968 | 186.8377 | 31.1651 |
| Reverse pose 0 | 346,520 | 64,670,108 | 186.6273 | 31.1874 |
| Exact-static pose 0 | 346,520 | 0 | 0 | 0 |

Covered query histograms (`queries: receiver count`): cold `256:346514, 512:6`; forward `4:94936, 256:250940, 512:7`; reverse `4:95399, 256:251115, 512:6`; static `0:346520`. All uncovered pixels have zero queries. Six/seven/six equal-depth duplicate invocations explain the 512-query pixels; they contribute 1,536/1,792/1,536 queries beyond one invocation per final receiver. Hidden-behind-prepass invocation counts are 499,264/499,748/499,264/499,264, with **zero hidden queries**. Main and visibility depth agree bit-for-bit everywhere; no visible-depth overshoot occurs.

| Reset bit | Forward receivers | Reverse receivers |
|---|---:|---:|
| 1 global, 2 settings, 4 cut, 8 overflow | 0 each | 0 each |
| 16 surface | 248,076 | 248,389 |
| 32 narrow | 2,635 | 2,517 |
| 64 moved | 0 | 0 |
| 128 contradiction | 236 | 215 |
| 256 outside volume | 0 | 0 |

Cold has bit 1 on every covered receiver; static has no reset bits. Forward actual query attribution is surface **63,509,248**, narrow **674,560**, contradiction **60,416**, reusable **379,744**. Reverse is **63,589,120 / 644,352 / 55,040 / 381,596** respectively. These are query counts, not additive GPU timings.

## Why the mandatory nearest anchor fails

These reason bits overlap. They describe the anchor, separately from skipped outer taps.

| Anchor reason | Forward | Reverse |
|---|---:|---:|
| Out of frame / clip | 0 | 0 |
| Zero history count | 266 | 147 |
| Sample offset mismatch | 277 | 87 |
| Normal dot fails `> .999` | 244,266 | 244,814 |
| Plane residual fails `< max(1e-5, footprint*.01)` | 189,553 | 189,623 |
| Distance fails `< footprint*4` | 2,062 | 2,162 |
| Valid anchor, insufficient outer weight mass | 1 | 2 |

Normal rejection participates in 98.46% / 98.56% of surface-reset pixels. Exact anchor masks include normal-only `8:58483 / 58751`, plane-only `16:3784 / 3568`, normal+plane `24:183486 / 183821`, and normal+plane+distance `56:1777 / 2010`. `summary.json` retains **every** overlapping mask count for anchors and outer taps, plus all query/reset histograms. Outer rejected-tap totals are 1,368,485 / 1,373,591; these are not additional anchor failures. Static performs **zero fetches**, directly copying the prior visibility/guide. Its eight retained attachment words equal the reverse frame exactly.

`gpu-operands.json` preserves 31 actual GPU operand records per moving frame, sampled from failing anchors (246 / 238 eligible hashed pixels; bounded first-31 capture, not a statistical sample). Examples from forward motion:

| Pixel | Dot | Plane residual / threshold | Distance / threshold | Failure |
|---|---:|---:|---:|---|
| (767,538) | .99443543 | .000116264 / .000125175 | .002787746 / .050069869 | normal only |
| (656,604) | .50643522 | .002909936 / .000074742 | .004225779 / .029896764 | normal + plane |
| (608,615) | .97034246 | .043298699 / .000150575 | .095690340 / .060230125 | normal + plane + distance |

All three have history count 256 and matching offsets (0.5,0.5). For (767,538), current GPU world position is `[-2.147313356, .134213552, -.199217379]`; previous anchor position is `[-2.145980597, .136311963, -.200479105]`. Full current/previous positions, normals, offsets and thresholds remain in every record. Independent CPU f64 reconstruction from retained f32 depth differs from the recorded GPU previous positions by at most 1.30e-6 across the 62 records; decoded prior normals differ by at most 1.53e-7. CPU reconstruction is labeled separately and is not substituted for GPU operands.

The four-frame reason sheet and both full-size moving anchor overlays were inspected. Surface rejection spans the quarry interior, including the broad floor and central blocks, rather than concentrating at the image boundary. These are diagnostic visibility/reason overlays, not new soft-quality images. Reset colors: purple cold, red surface, amber narrow, pink contradiction, green reusable, cyan static cache. Anchor colors: red normal, amber plane, purple both, blue additional causes, green accepted.

## Verification, synchronization limits and disposition

Khronos is active: `errors=0 missing_layer=0 active_layer=1`. The cold GPU mesh interval is 345.406 ms, observed completion 360.261 ms, and the completion/readback guard passes at 376.445 ms. Moving diagnostic GPU intervals are 212.523 / 214.410 ms; cached is 6.512 ms. **Validation and per-query atomics are enabled: these are not production costs.** Storage side effects can affect early depth execution. No validation/device failure or second run occurred.

Cold is drained for the stop guard. The other three frames use ordinary admission, reaching one/two/three outstanding renderer flights, with a diagnostic compute/copy submitted between renders to preserve each frame before overwrite. No moving per-frame wait was introduced. Copies still perturb the queue; there is no paired drained-versus-pipelined timing experiment and no synchronization-bug conclusion. Cold and motion also use different history ages from the 24-frame cost blocks. Prior 138–140 ms production timings remain unchanged evidence; this run identifies the dominant work path, not their exact instruction-time breakdown.

RTX 4070 SUPER, NVIDIA 617.14, Ryzen 7 7800X3D. Preflight: 15% reported CPU load, 11.43 GiB available RAM, 26.39/163.09 GiB free C:/D:, GPU 9% with 3,298 MiB already used; no competing renderer/Cargo process. Applications remained open. Two Cargo jobs, offline/locked, release lib test only. Existing test/example executables were preserved and all local compiler inputs touched before warm-target reuse. Build **27.576 s**, native verification **18.450 s**. Recorded instrumentation-to-build interval **164.84 s** excludes earlier reading, candidate application and script drafting; CPU analysis/packaging is additional. A NumPy unsigned-mask analysis error and correction are preserved without rerunning the GPU.

GPU and shared-target slots were explicitly **released at 2026-11-04T06:31:00.227547Z**, before packaging; direct checks find no owned process remaining. The shared target contains the diagnostic test binary: its next owner must touch/rebuild their own inputs. Accepted checkout source was restored after freezing exact source and patches.

**Next bounded direction:** establish whether a few failing nearest anchors represent different scan triangles/planes or incorrect same-surface guides, using the retained operand pairs to select cases. Any subsequent correction should target correspondence or guide correctness while preserving the rejection thresholds and original image gates. The present evidence does not justify loosening tolerances, tuning narrow resets, changing synchronization, or replacing the architecture.

## Reproduction and evidence

### Root source-geometry follow-up

Root audited all 183 evidence hashes and inspected the complete moving-anchor overlay. A separate CPU-only pass checked four recorded failing pairs against the original 13,034,903 source triangles. Each of the eight recorded GPU positions has one containing source-triangle candidate within a predeclared 20-micrometre plane/bounds tolerance and 0.0001 barycentric tolerance. Selection uses position only; normals are compared afterward. All pairs use different triangles of mesh zero.

Source geometric normals, oriented toward the unchanged light, align with the recorded current normals by at least 0.99999993 and with the quantized prior normals by at least 0.99996183. The actual source-plane angle changes are approximately 5.57, 59.24, 144.49 and 13.86 degrees. These examples support real microfacet changes, not a normal decoding or transform fault. The CPU calculation does not reproduce raster snapping or independently prove GPU primitive identity in a depth tie; it is limited to these four operand-selected pairs.

The source pass took 6.03 seconds without a build or GPU run. Its script, exact points, candidate vertices/barycentrics, comparisons and input hash are preserved at `D:/Coding/renderlab-work/m6-history-surface-correspondence/`. The computational tolerances locate source triangles; they do not change any rendering or history-rejection tolerance. The next mechanism must address visibility reuse across fine geometry rather than treating these measured normal differences as corrupted data.

Durable private bundle: `D:/Coding/renderlab-work/m6-history-quarry-resets-t3-98c26bcb/`; original local evidence: `.refs/m6-history-quarry-resets/`. No source assets are included. `run.py` records the exact command/environment; `analyze.py` reproduces summaries/overlays CPU-only. `candidate-with-diagnostic.patch` applies to `65d0f2d`; `diagnostic-only.patch` applies after the measured cost candidate. Supply the preserved `diagnostic.wgsl` override. No diagnostic source is promoted.

The 22-word readback preserves original words 0–7 (value/guide). Words 8–21 are queries, reset mask, shaded invocations, **anchor mask**, hidden invocations, depth overshoots, equal-depth invocations, cached invocations, **outer mask**, **outer rejected taps**, **fetch invocations**, hidden queries, visibility-depth bits, main-depth bits. Bold fields replace only private diagnostic slots. A 4 KiB tail holds an eligible-record counter and 31×32-word GPU operand records. No additional large counter plane was added; queue-ordered per-frame readbacks preserve all four outputs.

| Artifact | SHA-256 |
|---|---|
| Diagnostic binary | `b84b0f54695d46dbfa1c0b8a1bf0f85756920d7627afaa7ee6781e2fd363fcfc` |
| Full diagnostic candidate patch | `081382183578b07cf0febb7aeaead0006a5de1b5d7d8f323334d5a27693b7128` |
| Diagnostic-only patch | `80d8e43efd3170faa73715af76cf86a0cdb29de1b38c8cae2d27bb53d16100df` |
| Compiler-input manifest | `e1399918f9d4a78dc969d834bb71a62bf96c8d5e37a3d354727f4ae09699837c` |
| Moving diagnostic WGSL composition | `999cc6b63a815123a350cfe48015288fc6393f82834f4b1381b3334d7155af73` |
| Cached diagnostic WGSL composition | `a538b6104fb66f54d71c09b2c01b55014518fa516a3275f4b0af7a6de4f91c6c` |
