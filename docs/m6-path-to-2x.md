# M6 path to 2x Unreal

Design decision, 2026-11-04. This is CPU analysis of existing evidence only: no renderer change, GPU run or build. The Renderlab inputs are the [fresh-compute cost](m6-fresh-compute-cost.md), [raster-share](m6-raster-share-cost.md), [reduced-ray gate](m6-reduced-ray-gate.md), [hardware-shadow](m6-hardware-shadows.md) and [history-cost](m6-history-quarry-cost.md) records. The Unreal inputs are the preliminary `D:/Coding/renderlab-work/ue-m5/Quarry/` runs: the `launches/*profile2*/` ProfileGPU trees, CSV summaries and logs, plus the import and geometry-audit records and `main.png`. Labels: **M** is a measured value read from those records; **I** is an inference.

## Decision

1. **The preliminary Unreal frame is not a matched denominator.** In every profile2 run, Unreal rasterized, shadowed and ray-traced the 131,453-triangle Nanite fallback. It rendered no full-detail Nanite geometry (the evidence is below). Its 1.7-2.1 ms buys about 1% of the scan's triangles, so the "7x" gap mixes a work gap with a fidelity gap. No 2x target number exists until Unreal renders Nanite at full detail.
2. **Renderlab's M6 numbers use the full-resident path.** `fresh_shadow_tests::load_quarry` returns `prepared::Prepared`, and the screens draw it with `Visibility::Frustum`. Each geometry pass submits 13,034,903 triangles in twelve draws. The M4 virtual path, which ends the 338-pose path at 3,981,802 triangles, is not involved. Hardware and fresh shadows reject virtual geometry.
3. **Retained direction, in order:**
   1. **Traced primary visibility.** Replace both 13 M-triangle raster passes with one camera ray per pixel against the existing full-detail TLAS, followed by deferred compute shading from the hit record. This is the [first unit](#4-first-unit-traced-primary-visibility). It is needed under every denominator and every shadow method.
   2. **Matched Unreal measurement:** Nanite actually rendering and ray-traced, with matched exposure. Measure its frame cost and its shadow-visibility error against the existing 16,384-ray reference. That error defines a Level-B gate.
   3. **A Level-B soft-shadow method** at Unreal-like ray counts: a few stratified rays per receiver plus penumbra-bounded reconstruction. It is stateless at first.
   4. The 256-ray fresh estimator stays as **the reference and an opt-in exact mode**. It is not the 2x path.
4. **Cluster LOD and virtual shadow maps are deferred, not adopted.** Cluster LOD is the fallback if traced primary visibility misses its budget. It is still needed later for larger-than-memory worlds and to bound acceleration-structure memory.
5. **Feasibility.** Exact Level-A soft shadows cannot reach 2x on this GPU. The Level-A detection floor alone is about 2.0 ms of rays, which puts the best exact frame near 2.6 ms; 2x would need a matched Unreal frame of at least 5 ms (I). 2x is plausible only with Level-B shadows of about 0.35 ms or less **and** a matched Unreal scene render of at least about 2 ms. If matched Unreal stays near the fallback's 1.6 ms scene render, 2x needs about 0.8 ms. That is not plausible here without Unreal's own trade-offs: temporal accumulation or cached light-space maps.

## 1. Where the 7x goes

Renderlab: process-2 medians from the fresh-compute screen, at 1x with 256 rays and the RLE2 environment. Process medians span 14.6-22.6 ms. Unreal 5.7.4-51494982: CSV medians over 64 frames (rows 96-159) at pose 0, 1920x1080, no AA, 100% screen percentage. VSM uses SMRT with 8 rays and 8 samples. HRT uses 1 spp, the denoiser and temporal accumulation. All values are milliseconds.

| Stage | Renderlab | Unreal VSM | Unreal HRT | Work difference | Limiter |
|---|---:|---:|---:|---|---|
| Camera geometry | 3.01 receiver raster | 0.17 (prepass 0.046, base 0.085, anisotropy 0.041) | 0.18 | 13.03 M vs 0.131 M triangles (99x); 37.6 vs at most 0.38 triangles per covered pixel (M) | Triangle submission (I, from the raster-share screen: removing hidden fragments gave no reliable gain) |
| Second geometry pass and shading | 3.49 | Lights 0.03 (deferred) | In the shadow row | Renderlab submits 13.03 M triangles again | Same geometry, submitted twice (M) |
| Fixed Nanite overhead | none | 0.28 (vis buffer 0.172, base 0.057, readback 0.048) | 0.28 | No Nanite geometry drawn | Unreal only |
| Shadow visibility | 8.08 compute | 0.45 (VSM depths 0.312, SMRT projection 0.141) | 0.56 (lights with 1-spp trace 0.195, denoiser 0.309, TLAS 0.051) | 256 exact rays per receiver against 13 M triangles, vs 8 SMRT rays or 1 ray plus a denoiser against 0.13 M | Ray count times near-receiver traversal. M: 94% of the dispatch is traversal, cost is linear in rays, and capping rays at 25 cm keeps 0.917 of the cost. I: BVH memory locality |
| Tone and post | 0.06 | 0.04 | 0.04 | | |
| UI and unattributed | none | 0.60 (Slate 0.289, unaccounted 0.310) | 0.59 | | Unreal only |
| **GPU frame** | **14.65** | **1.93** | **1.91** | **7.6x** | |

Small passes are omitted (HZB, light grid, GPU scene and virtual-texture update, each at most 0.034 ms median), so the rows do not sum exactly. The 0.742 ms `VirtualTextureUpdate` in the VSM ProfileGPU frame is a single-frame spike against a 0.034 ms median, so attribution uses CSV medians and the trees only for structure. In motion, Unreal measures 2.62 ms (VSM, depths 0.433) and 2.07 ms (HRT). Its other static poses measure 1.69-2.14 ms.

Of Renderlab's 12.7 ms excess, about 6 ms is camera geometry submitted twice and about 7.6 ms is shadow rays. Unreal spends about 0.9 ms on UI, unattributed time and fixed Nanite overhead that Renderlab does not pay.

Unreal's frame is CPU-bound in this configuration, a Development editor build launched with `-game`: `FrameTime` 4.06 ms, game thread 2.25 ms, GPU 1.93 ms. An end-to-end throughput comparison needs a cooked Test or Shipping build.

### What Unreal rendered

- `PrePass`, `BasePass` and `RenderAnisotropyPass` each issue 12 conventional `ParallelDraw` draws totalling exactly 131,453 primitives. That is the sum of the twelve fallback LOD0 counts in `runs/views-02/geometry-audit.json`, 52,555 through 10,933. These passes never draw a Nanite proxy, so all twelve placements had non-Nanite proxies. `RHI/PrimitivesDrawn` is 408,997 per frame: three times the fallback, plus UI.
- `NaniteBasePass` contains only `Nanite::EmitDepthTargets` and no material shading pass. `Nanite::VisBuffer`, at about 0.17 ms, is fixed overhead.
- The VSM run draws the fallback casters in `RenderVirtualShadowMaps(Non-Nanite)`. In the HRT run, `RayTracingGeometry/TotalResidentSizeMB` is 6.6 MB, whereas Renderlab's acceleration structures for the full scan observe 911.8 MB. The rays traced the fallback.
- The log warns `Material /InterchangeAssets/gltf/M_Default ... missing bUsedWithNanite=True! Default Material will be used in game.` That is the likely cause, but it is not verified. The import records show Nanite enabled, keeping 100% of triangles, with fallback relative error 1.0.
- `main.png` has faceted cliff, slab and wall silhouettes compared with Renderlab's `m6-compute-cost/images/quarry-c2-pose-0.png`, and it is overexposed (a known caveat).

## 2. Target and per-pass budget

**Denominator.** Matched Unreal is unmeasured. It should cost at least the fallback frame's scene work, about 1.6 ms without UI (I). Full-detail Nanite raster, Nanite VSM casters and Nanite ray tracing replace about 0.17 ms of conventional raster and add their own work, while the fixed Nanite overhead is already paid. The plan uses **2.0 ms** as a deliberately strict working value, giving a Renderlab target of **1.0 ms**. Claims must compare against Unreal's scene GPU span excluding UI, which is the stricter choice, and report the full `GPUTime` beside it.

| Pass | Now (M) | Budget | Mechanism | Basis |
|---|---:|---:|---|---|
| Primary visibility | 3.01 | 0.25 | One closest-hit camera ray per pixel against the existing TLAS | I: 88.7 M shadow rays take 8.08 ms (11 G rays/s); a frame has 2.07 M primary rays, most of them background |
| Shading | 3.49 | 0.25 | Deferred compute from hit records with analytic texture derivatives | I: Unreal's base pass plus lights cost 0.12 ms; this removes the 13 M-triangle re-raster |
| Shadow visibility | 8.08 | 0.35 | Level B: about 4-8 stratified rays plus penumbra-bounded reconstruction | M: 0.032 ms per ray per covered receiver; Unreal spends 0.45-0.56 ms |
| Tone | 0.06 | 0.06 | Unchanged | M |
| Barriers and classification | none | 0.09 | | |
| **Total** | **14.65** | **1.00** | | |

Predicted after step 1 alone (I): the exact mode costs about 0.5 + 8.08 ≈ 8.6 ms, against 9.5-9.8 ms measured for hard 4x. Hard shadows at 1x fall from 4.2 ms to about 0.6 ms. Neither matches Unreal's 4.584° source, so neither is a parity claim.

**Sequence.** Each step is retained only if it passes its gate.

1. Traced primary visibility and deferred shading ([section 4](#4-first-unit-traced-primary-visibility)).
2. The matched Unreal measurement ([section 5](#5-missing-matched-unreal-steps)). It runs concurrently in the UE worker's private project; the two steps share no files.
3. A Level-B gate: the existing G1-G7 checks and reference, with thresholds taken from Unreal's measured error on the same receivers and poses. The faint-shadow (G6) and contact enumerations stay mandatory, so their losses are visible even where Unreal shares them.
4. A Level-B shadow candidate. The first candidate gives each receiver k rays, an interleaved subset of one jointly stratified set per 2x2 quad. It reconstructs with a kernel limited by the hit-distance penumbra estimate, so narrow contacts are not smeared, and it keeps no history. k is chosen after step 3.
5. Moving-light and motion comparisons. Ray-traced cost does not depend on light motion, while Unreal's VSM must re-render invalidated pages (I). A moving sun is therefore the most plausible 2x case, and a static light with a static camera is the hardest.

If Level B demands near-exact penumbrae, 2x on this GPU needs one of three things: (a) about 2-3x the ray throughput; (b) ray reuse across frames, as Unreal does, which would be a new design rather than the rejected nearest-anchor history; or (c) cached light-space caster maps, which require caster LOD.

## 3. Rejected and deferred options

| Option | Status | Reason |
|---|---|---|
| Cluster LOD raster at 1 px error as the first step | Deferred; this is the fallback | M4's 1 px cut is 3,981,802 triangles. Unlit at 4x it measures 3.57 ms GPU p50 against 5.21 ms for full geometry ([scan results](scan-results.md)), so LOD alone does not pay. It would also need GPU culling and selection, a visibility buffer and a fix for receiver/BVH mismatch. At pose 0 one pixel is about 7.3 mm at 8.7 m, against the 1 mm normal bias, so LOD receivers would self-shadow against the full-detail BLAS. Traced primary visibility needs none of this. |
| Raster visibility buffer over all 13 M triangles | Rejected | It removes one pass, but its about 3 ms floor exceeds the whole budget |
| Depth sharing (Equal, first-winner blending or depth seed) | Rejected, measured | [Raster-share screen](m6-raster-share-cost.md): exact, but no reliable gain |
| Shadow maps or VSM for umbra and bulk, rays only for penumbra, in exact mode | Rejected | Capping rays at 25 cm keeps 0.917 of the ray cost. Near the receiver, the receiver's own surface defeats conservative map certification, so savings are at most about 8%. Rasterizing 13 M casters costs about 3 ms per map: three 4096 maps with 27 M caster triangles took 6.2-10.1 ms. This stays as Level-B alternative B2 if B1 fails, and needs caster LOD plus a cache. |
| 64-detection-ray adaptive tracing | Rejected on GPU | Misses faint speck shadows ([gate record](m6-reduced-ray-gate.md#first-gpu-result-2026-11-04)) |
| Any Level-A method as the 2x path | Not the 2x path | The 25% detection floor is about 2.0 ms of rays |
| Earlier history reuse | Rejected, measured | 72% of receivers reset under motion; 138 ms. Unanimous-solid-support (USS) reuse missed partial shadows |
| Software micro-triangle raster | Not needed | Traced visibility costs nothing per triangle submitted |

## 4. First unit: traced primary visibility

The scope is regular full-resident geometry on the hardware path, at 1x, with opaque double-sided materials and a directional light. Virtual geometry and 4x keep raster. It adds no new acceleration structure: it reuses the TLAS and BLASes over the existing vertex and index buffers.

**Phase A: probe.** No production change.

- Add one compute dispatch that traces a closest-hit ray through each pixel centre. It unprojects near and far through the same camera matrix, as the existing source check does. It writes depth, instance, primitive and barycentrics at the three registered poses.
- Time it in one process against the receiver raster, with 24 alternating pairs. Add processes only if the result lands near a threshold.
- Compare against the raster receivers. Enumerate every pixel where coverage, the winning instance and primitive, or the world position (by more than 0.1 mm) differs. Classify each as a silhouette sample, a depth tie (the known reversed-Z cases) or other.
- **Stop rule:** reject this route if the trace median exceeds 0.75 ms (three times its budget), or if any "other" mismatch remains unexplained. Then switch to GPU cluster LOD with a raster visibility buffer.

**Phase B: integration.**

- Deferred compute shading reads the hit records. It fetches vertex attributes from the existing shared buffers, which adds storage usage but no second copy. It uses perspective-correct barycentrics, the exact geometric normal and analytic ray-differential texture derivatives with the existing anisotropic sampler.
- It reuses the existing lighting, normal-map and environment WGSL, and reload stays transactional.
- Fresh and hard hardware shadows read the traced receivers.

**Acceptance evidence.**

1. The Phase A enumeration and paired timing.
2. Fresh soft shadows on traced receivers pass Level A G1-G7 against a 16,384-ray reference regenerated on those receivers with the same seeds. The ray origins change from derivative normals to exact triangle normals.
3. Full-image HDR comparison against raster production at three poses and the 1 cm step, with derivative-related differences quantified. Texture and silhouette error against the existing supersampled quarry reference must be no worse than raster at 1x, under the [scan-results](scan-results.md) metric.
4. The normal-map (mirrored and nonuniform), environment, material-assignment, reload and hardware-shadow GPU tests pass on the new path with validation active.
5. Paired per-pass timing using the fresh-screen method. Targets: primary visibility at most 0.4 ms, shading at most 0.4 ms, fresh span at most 9 ms, hard 1x span at most 1.0 ms.
6. Inspection of the complete images: silhouettes, contacts, crevices, rubble and motion.
7. Memory: the hit-record targets (about 16 bytes per pixel). The acceleration structures stay at 911.8 MB.

## 5. Missing matched-Unreal steps

1. **Make Nanite render.** Give the quarry materials Nanite usage, either by fixing the Interchange parent or by reparenting to a project-owned master material with `bUsedWithNanite`, then resave. Verify in ProfileGPU that no conventional 131,453-primitive draws remain and that Nanite material shading appears. Check visible triangles with `NaniteStats`, and confirm the silhouettes match Renderlab's full-detail image.
2. **Ray-trace full detail.** Enable Nanite ray tracing (`r.RayTracing.Nanite.Mode 1`) and confirm the ray-tracing resident size reflects full detail. Otherwise, record the fallback as an Unreal fidelity limit.
3. **Exposure.** Fix the main-view mapping, and compare HDR rather than the clipped PNG.
4. **Environment.** Renderlab's timing includes RLE2 and Unreal's has none. Either time Renderlab with the environment black or add a matched environment.
5. **Visibility error.** Capture shadowed and unshadowed HDR pairs from the main view at the three poses, both on the first frame after a cut and in steady state. Score them against `m6-adaptive/gpu-phase-quarry/quarry/` with G1-G7. The existing SceneCapture EXRs show fallback geometry and cannot be used.
6. **Timing.** Use a cooked Test or Shipping build. Report scene GPU without UI alongside `GPUTime`, the static, camera-motion and moving-sun cases, the CPU critical path, async-queue use (0.06 ms here) and the VSM cache state.

## 6. Open risks

- **Primary trace cost is unmeasured.** Twelve instances overlap and closest-hit rays cannot stop early. Phase A decides.
- **Matched Unreal may land near 1.6-2 ms.** 2x then needs 0.8-1.0 ms, with about 0.2 ms left for shadows.
- **Level B is unknown until measured.** If Unreal reaches its quality through temporal accumulation, a stateless method needs more rays or a new reuse design.
- **The fast path is RT-only.** It depends on experimental wgpu ray queries and RT hardware. At 4x it would trace four rays per pixel, and raster remains everywhere else.
- **Acceleration-structure memory scales with source triangles.** It is 911.8 MB for this scene (about 70 B per triangle). Larger-than-memory scenes still need LOD BLASes or streaming.
- **Shading will not be bit-identical to raster.** Analytic and quad derivatives differ slightly, so acceptance rests on reference error and inspection.
- **1x aliasing is unchanged.** Both paths sample pixel centres. The Unreal comparison must fix the same AA configuration on both sides.

Evidence for this decision is the cited records and the Unreal launch directories listed above. The work was CPU-only reading and arithmetic, not stopwatched, and no implementation time applies.
