# N8: virtual sun shadow map plus an exact local test (non-RT Interactive)

Implementation unit, 2026-11-06 UTC, base `437cbd5`, branch `t3/non-rt-virtual-shadow-path`. Bar (Islam, 2026-11-06): at least Unreal's quality on every measure at Unreal-like cost, better where cheaply possible; film quality is the long-term target, not this unit's gate. Labels: **M** measured here, **I** inference.

## Result

**Quality: N8 beats Unreal VSM on almost every measured quantity on all four benches, but misses on two counts. Cost: about VSM's on S2, 1.4-1.7x VSM on S3, and 1.6-8x on the 13 M-triangle quarry; a moving sun over the quarry costs 16-17 ms. Not film quality and not the 2x-faster floor.**

- **Contacts.** Lost contacts fall from Unreal's 2-12k per phase to 0-106 (S1-S3); lost umbra edges on the quarry fall from 11.9k to 0.9k.
- **Partial error.** Partial RMS is 0.08 (S1), 0.074-0.080 (S2), 0.106-0.113 (S3) and 0.30 (quarry), against VSM's 0.16, 0.17-0.18, 0.18-0.19 and 0.48. On S1 and S2 this is at RT Interactive's level (same 8 directions, exact rays).
- **Flips and leaks.** Solid flips are lower than VSM's everywhere (S2 0-6 against 119-167; S3 46-69 against 187-226; quarry 6.5k against 12.4k). Umbra leaks are lower on S3 and the quarry.
- **Misses against Unreal:**
  1. False shadows on lit quarry receivers: 8.5k against VSM's 87. VSM's normal offset avoids them at the price of the lost contacts above.
  2. Umbra leaks: S1 3 against 0 (single directions at a penumbra-to-umbra boundary), and S2 up to 150 per frame against 62, but only in the frames where the thin reveal plate moves (13 at rest). The plate case is not diagnosed.
- **Cost (M, RTX 4070 SUPER, development build, Vulkan validation layer on):**
  - **S2:** shadow p50 0.37-0.45 ms against VSM's depth plus projection 0.35 ms. Whole frame 0.51-0.67 ms against 2.1-2.2 ms.
  - **S3:** 0.60-0.67 ms static and holds, 0.84-0.85 ms with the sun turning, against 0.36-0.37 and 0.59 ms. Whole frame 0.78-1.03 against 2.2-2.4 ms.
  - **Quarry:** 1.8-4.0 ms static over three runs (0.82 ms without the ring walk) against VSM's 0.50 ms. A turning sun re-rasterizes all 13 M triangles into 3 levels x 2 layers every frame: 16-17 ms, against VSM's roughly 0.95 ms (frame 2.06 against 1.61 ms static).
  - Whole-frame comparisons favour Renderlab mostly because the primaries are still traced.
- **Not yet a non-RT frame.** The shadow pass uses no ray query and no acceleration structure, but its receivers and `(placement, primitive)` records still come from traced camera rays. A raster visibility buffer with the same records (N1b) is needed for a frame with no RT at all.

### Benchmarks (M)

Unreal rows are the recorded native results (S1: [bench-contact-fold.md](bench-contact-fold.md); S2/S3: `E:/renderlab-work/motion-benches/S*/comparison.csv`, [bench-motion.md](bench-motion.md)); their raw arrays were compacted, so they are not rescored. The film references were regenerated in this run, and RT Interactive was rerun as a control: it reproduces its recorded scores to 4 decimals on S1, S2 and S3, which validates the regenerated references. Contacts are pixel-frames per phase; leaks and flips are the worst frame. "Shadow ms" is N8's page passes plus lookup, and Unreal's ShadowDepths plus ShadowProjection.

**S1, contact/fold/stacked occluders** (static, 2 poses x black/environment; N8 is identical across environments):

| Mode | Contact loss | Leaks | Partial RMS | Solid flips |
|---|---:|---:|---:|---:|
| N8, pose 0 | **0/419** | 3 | **.0828** | **0** |
| RT Interactive, pose 0 | 0/419 | 0 | .0878 | 0 |
| UE VSM, pose 0 (black / env) | 5/419 / 6/419 | 0 / 0 | .1642 / .1644 | 0 / 1 |
| N8, pose 1 | **0/335** | 3 | **.0809** | **0** |
| RT Interactive, pose 1 | 0/335 | 0 | .0868 | 0 |
| UE VSM, pose 1 (black / env) | 2/335 / 3/335 | 0 / 0 | .1628 / .1620 | 0 / 0 |

S1 timings in this run are not usable: both RL modes measured 1.9-3.1 ms against Interactive's recorded 1.54 ms, from about 8 frames each.

**S2, moving slivers, disk 0.04 rad:**

| Case / phase | Mode | Partial RMS | HDR RMS | Lost contacts | Max leaks | Max flips | Frame p50 ms | Shadow p50 ms |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| camera 0.25 px, out | **N8** | **.0790** | .0106 | **106**/19,396 | 150 | **4** | 0.67 | 0.45 |
| | RT Interactive | .0749 | .0105 | 0 | 0 | 0 | 0.30 | - |
| | UE VSM | .1752 | .0151 | 2,101 | **62** | 167 | 2.13 | 0.35 |
| camera 0.25 px, hold | **N8** | **.0799** | .0113 | **64**/11,264 | **13** | **2** | 0.59 | 0.37 |
| | UE VSM | .1758 | .0154 | 1,039 | 57 | 152 | 2.12 | 0.35 |
| camera 0.25 px, return | **N8** | **.0778** | .0102 | **87**/17,990 | 151 | **6** | 0.66 | 0.44 |
| | UE VSM | .1739 | .0145 | 1,972 | **62** | 155 | 2.13 | 0.35 |
| shadow step, out | **N8** | **.0745** | .0099 | **64**/16,687 | **2** | **0** | 0.60 | 0.42 |
| | UE VSM | .1697 | .0152 | 2,070 | 65 | 120 | 2.18 | 0.36 |
| shadow step, hold | **N8** | **.0739** | .0099 | **64**/8,832 | **0** | **0** | 0.51 | 0.33 |
| | UE VSM | .1675 | .0150 | 1,059 | 66 | 119 | 2.13 | 0.35 |

**S3, 525 k-triangle slab and pillars, 15° sun:**

| Phase | Mode | Partial RMS | HDR RMS | Lost contacts | Max leaks | Max flips | Frame p50 ms | Shadow p50 ms |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| static | **N8** | **.1060** | **.0154** | **0**/48,512 | **0** | **48** | 0.86 | 0.66 |
| | RT Interactive | .0775 | .0142 | 0 | 0 | 0 | 0.31 | - |
| | UE VSM | .1878 | .0194 | 2,929 | 111 | 187 | 2.22 | 0.37 |
| rotate 0.25°/s | **N8** | **.1072** | **.0156** | **14**/194,292 | **0** | **62** | 1.03 | 0.84 |
| | UE VSM | .1875 | .0194 | 11,630 | 111 | 190 | 2.43 | 0.59 |
| rotate 1°/s | **N8** | **.1073** | **.0156** | **56**/196,290 | **0** | **69** | 1.03 | 0.85 |
| | UE VSM | .1863 | .0200 | 12,289 | 106 | 192 | 2.44 | 0.59 |
| jump recovery | **N8** | **.1129** | **.0156** | **0**/51,777 | **0** | **46** | 0.87 | 0.67 |
| | UE VSM | .1811 | .0219 | 3,153 | 114 | 226 | 2.22 | 0.37 |

N8 responds to the 5° sun jump in the jump frame itself (no history).

**Quarry, 13 M triangles, poses 0 and 1.** The frozen Level B reference (`m6-adaptive/gpu-phase-quarry`) predates today's traced receivers: RT Interactive scores .304 against it with a -.18 partial mean. So this run builds a new reference on the current traced receivers, in the same process, as `gpu_primary_traced_quarry_acceptance` does (256-ray Exact seeds 1-64, 16,384 rays per pixel). It scores RT Interactive at .080, the documented production level. Unreal's decoded native VSM masks (`native-vsm-baseline-p0-02`, `-p1-01`) are scored against the same reference on the pixels where both are valid. The Renderlab rows are unchanged on that common domain.

| Pose | Mode | Partial RMS | Partial mean | G1 violations | G4 RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | **N8** | **.298** | -.037 | **9,888** | **.151** | **6,470** | **2,528** | 8,477 | **901** |
| 0 | N8, own triangle only (`ring: false`) | .356 | +.007 | 10,318 | .167 | 11,817 | 4,442 | 13,169 | 2,042 |
| 0 | RT Interactive | .080 | .000 | 8,139 | .033 | 0 | 0 | 0 | 0 |
| 0 | UE VSM native | .483 | +.350 | 11,019 | .351 | 12,390 | 14,935 | **87** | 11,884 |
| 1 | **N8** | **.296** | -.036 | **9,760** | **.149** | **6,426** | **2,512** | 8,433 | **893** |
| 1 | N8, own triangle only | .355 | +.004 | 10,196 | .167 | 11,773 | 4,440 | 13,123 | 2,033 |
| 1 | RT Interactive | .081 | .000 | 8,082 | .033 | 0 | 0 | 0 | 0 |
| 1 | UE VSM native | .482 | +.349 | 10,943 | .355 | 12,500 | 15,048 | **88** | 11,992 |

Lost umbra edges: reference umbra receivers within 3 px of a lit or partial receiver that the candidate lights by more than half (23.4 k such receivers). Lit darkened: reference-lit receivers below 0.9.

**Quarry cost:**

| Mode | Shadow p50 / p95, static (ms) | Camera alternating poses 0/1 | Sun turning |
|---|---|---|---|
| N8 | 1.77 / 2.33, then 3.68 / 6.82 and 4.04 / 6.02 in two later runs | 1.85, 2.59, 2.42 | 17.4, 16.9, 17.2 |
| N8, own triangle only | 0.80 / 1.02, 0.81 / 1.06, 0.82 / 1.18 | 0.92-0.93 | 15.9-16.0 |
| RT Interactive | whole frame 0.95 ms; receivers and 8 rays fused, 0.55 ms | 0.94 | - |
| UE VSM (recorded) | 0.50 (0.349 depth + 0.155 projection) | - | about 0.95 |

The first run's 1.77 ms is the outlier; it is not explained. All three ran the same shader code; the later builds changed only comments and test code.

### Misses, honestly

- **Not film quality.** N8's partial error is 1.4x (S3) to 3.7x (quarry) RT Interactive's, which itself is below the film bar. It is never a default, and is not presented as meeting the film gate.
- **Not at Unreal's cost on large scan content.** On S3 the lookup over about 2 M covered pixels costs 1.8x VSM's projection plus depth. On the quarry the vertex-fan walk over rough scan rings costs 1-3 ms. A turning sun re-rasterizes everything, because there is no caster LOD (Nanite clusters, in Unreal) and no static/dynamic page split.
- **Lit-receiver false shadows on the quarry** (8.5k against 87): texel-scale silhouette quantization at crests and occluders hidden below another surface in light space.
- **S2 thin moving plate**: up to 150 umbra receivers per frame get 1-2 of 8 directions lit while the plate moves.
- **Memory is dense**: 2048² x (4 + 4 + 4) bytes per level, 159-203 MB for 3-4 levels. That is Unreal's page-pool scale, without a physical pool.

### Next levers (I)

1. **Skip the ring walk where it cannot matter.** Precompute, per triangle, the highest elevation its 1-ring reaches above its plane (8 B per triangle, about 104 MB on the quarry). The lookup skips the walk whenever the sun's disk is above that horizon. The quarry has 0.82 ms without the walk; the walk still wins every quality measure there, so it should be kept where it matters, not removed.
2. **Caster LOD for the page raster**, from the existing cluster LOD (virtual geometry), with the error bounded by the texel size. A turning sun is 16-17 ms at full detail.
3. **Clipmap panning by page-address offset** instead of window re-centering, and a static/dynamic page split, as Unreal does.
4. **Wave-level early-out**, as SMRT's adaptive ray count: stop marching a direction when the whole wave is lit after the first direction.
5. **Feed the same kernel from a raster visibility buffer (N1b)**, for a frame with no RT.

## Why Unreal is cheap, and what N8 copies from that

Unreal's non-RT sun shadow is cheap because it does almost no per-pixel geometry work:

1. **It rasterizes, and rarely.** Shadow depth is rasterized into 128-texel pages of a 16K virtual clipmap, only for pages that visible pixels request, and a rendered page stays cached until the light turns, the clipmap pans past it or an instance whose bounds touch it moves. With Nanite the casters are LOD'd clusters of about a texel each. On S3 (Nanite off) VSM depth costs 0.27 ms static and 0.50 ms with the sun turning (M, [bench-motion.md](bench-motion.md)).
2. **Per pixel it only reads a texture.** SMRT traces up to 8 rays of up to 8 depth samples each through the one depth layer, and adaptive ray counts stop after the first ray when a whole wave is lit or shadowed. S3's projection costs 0.11 ms for the whole frame.
3. **Contacts are traded away.** A normal offset of several millimetres and a screen-space start push the lookup off the surface, so contacts vanish (31% lost on the quarry) and thin features leak.

Renderlab's RT path instead traces exact rays against 13 M millimetre triangles. Without RT cores that costs 8-9 ms (N0, N4) and the N5 split projected 5.3 ms at best. N8 therefore takes Unreal's structure for everything beyond a few millimetres and spends exact work only where Unreal fails, at contacts:

| Part | Unreal VSM | N8 |
|---|---|---|
| Far field | Cached virtual clipmap pages, SMRT march | Same idea: page-driven, cached clipmap (two layers + triangle ids), SMRT-style march |
| Near field | Normal offset + screen trace (no geometry) | Exact watertight test of the receiver's own triangle and vertex 1-ring, over the whole ray |
| Per-sample decision | Depth compare with slope bias and history extrapolation | Depth compare as a candidate filter, then an exact test of the texel's top triangle |
| Light model | 8 SMRT rays, blue noise, TSR | Interactive's same 8 per-pixel R2 directions |

## What was studied (own words; no Epic or Unity code copied)

**Unreal 5.7.4** (installed source, read locally): `VirtualShadowMapProjectionDirectional.ush`, `VirtualShadowMapSMRTTemplate.ush`, `VirtualShadowMapSMRTCommon.ush`, `VirtualShadowMapPageMarking.usf/.ush`, `VirtualShadowMapProjectionCommon.ush`, `VirtualShadowMapCacheInvalidation.ush`, `VirtualShadowMapCacheManager.cpp`.

- *Level choice.* A clipmap level is chosen from the log2 distance to the clipmap origin plus a resolution bias, so texel size grows with view distance. Marking uses the same function, so pages exist where they are sampled; sampling falls back to the best mapped coarser page.
- *SMRT.* Each ray is marched from its far end toward the receiver at quadratically spaced times. The first valid sample is a plain depth compare; later samples count a hit only when the ray crosses the occluder depth within the step's own depth change, and a "behind" sample (a jump to a farther surface) is replaced by the last surface extrapolated with a clamped slope. This is a heuristic for a single depth layer: thin floating casters are not over-darkened, solid ones can leak. Ray length scales with view distance, and a texel dither plus a slope bias derived from the receiver normal hide aliasing.
- *Adaptive ray count.* After the first ray, if every lane of the wave missed, the loop ends; after a few rays, if every lane hit, it ends. Most pixels pay for one ray.
- *Cache.* Pages keep their content across frames; clipmap panning is handled by a page-address offset, not by invalidation; instance-bounds invalidation also marks pages not requested this frame; static and dynamic geometry can be cached separately; turning the light invalidates everything.

**Unity HDRP** (built-in package source under the Unity Companion License, read locally): `HDPCSS.hlsl`, `HDShadowSampling.hlsl`, `HDShadowAlgorithms.hlsl`, `HDCachedShadowManager.cs`.

- *Directional PCSS.* Samples are extruded on a cone toward the light (a sample's z is offset by its radial distance), so only casters inside the light's cone count and the receiver does not occlude itself. A clumped Fibonacci-spiral blocker search estimates average blocker depth, the penumbra width follows from it, and a PCF pass with a comparison sampler filters at that width, rotated per pixel and frame.
- *Caching.* Whole shadow maps (not pages) can update every frame, once, or on demand, in a cached atlas.

**What N8 adopts:** Unreal's page-driven clipmap with receiver-driven requests and a cache invalidated by light turn, window move and moved-caster bounds; SMRT's idea of marching each sun direction through the map with geometric steps; Unity's observation that per-direction samples must lie on the light cone so the receiver cannot shadow itself. **What it does differently:** the near field is exact; every map candidate is confirmed by an exact test of the texel's top triangle (the map is used as a spatial index, not as the answer); a second layer (the highest surface facing away from the sun) gives occluder thickness, replacing SMRT's slope-extrapolation heuristic; the directions are Interactive's, so the estimate is directly comparable with the RT path.

## What was built (M)

Files: `src/virtual_shadow_gpu.rs`, `shaders/virtual_shadow.wgsl`, `src/virtual_shadow_tests.rs`, `scripts/n8-{run,batch,quarry-score,fixture-view}.py`; small hooks in `src/shadows.rs`, `src/renderer.rs`, `src/primary_ray_gpu.rs`, `src/lib.rs`, the S1/S2/S3 harnesses and scorers.

- **API.** `SoftShadows::Virtual`, tuned by `Renderer::set_virtual_shadow_settings(VirtualShadowSettings)`: `near` r (default 2 mm), `texels_per_pixel_log2` (0: about one texel per pixel), `pages` per window side (16), `march_samples` (10), `finest_texel`, `bias_ulps`, `ring`. It requires `VIRTUAL_FEATURES` (primitive index, indirect first instance, per-vertex attributes; no ray query).
- **Clipmap.** Absolute levels `finest_texel * 2^k`, window of 16x16 pages of 128 texels (2048²) per level, centred on the camera in a sun basis that depends only on the sun direction. A receiver picks the finest level at about one texel per pixel whose window holds its march neighbourhood. Each level keeps the highest surface toward the sun (Depth24Plus, with an R32Uint triangle id) and the highest surface facing away from it, from the source winding.
- **Pages.** A mark pass requests the pages under each receiver's march neighbourhood; a compaction pass lists requested pages whose cache epoch is stale or which a moved caster's light-space rectangle touches; a stencil-masked raster renders all casters of a level only into those pages (zero-instance indirect draws when nothing is stale). Frames whose camera, sun, casters and windows are unchanged skip all of this and only run the lookup.
- **Near test.** The receiver triangle and every triangle sharing one of its vertex positions, found by walking vertex fans over the existing exact-position edge adjacency, are tested with N0's watertight test against all 8 directions over the whole ray. One axis permutation serves all 8 near-parallel directions; a cone cull skips triangles the 8 rays cannot reach.
- **Far march.** Per direction, 10 geometric steps from r to the highest caster (or the page reach). A sample is a candidate when the top surface is above the ray step (and above the receiver's own plane at the texel centre); candidates in the receiver's 1-ring are skipped (tested exactly already); otherwise the ray is tested exactly against the texel's top triangle and its three edge neighbours, and, for thick occluders, counted inside when it lies between the top and the back-face layer.

## Development screens (M)

Fixture: 960x540, 200x200-cell bumpy floor (1 cm cells, 2-6 mm relief), a box resting in it and a 2 mm wire, 8 directions compared with hardware Interactive on the same directions. Each screen was one guarded lease of 4-8 s.

"Shadow ms" is the GPU span from the first N8 pass to the end of the lookup; static frames skip the page passes, sun-motion frames re-rasterize every requested page.

| Step | 40° sun: pixels equal to RT / flips ≥ ½ / lit pixels darkened / mean abs | 15° sun: same | Shadow ms static / sun motion (40°; 15°) |
|---|---|---|---|
| First build: single layer, arrays passed by value | 99.6% / 417 / 19 / .0013 | 91.9% / 8,773 / 7,069 / .030 | 1.04 / - ; 1.40 / - |
| + back-face layer, precomputed ray structs | 99.6% / 5 / 448 / .0006 | 89.9% / 6,502 / 15,979 / .029 | 2.44 / 2.66 ; 2.37 / 2.50 |
| Per-invocation `var<private>` tables, static frames skip page work | same counts | same | 0.65 / 0.87 ; 0.73 / 0.88 |
| One axis permutation for all 8 directions | same | same | 0.57 / 0.79 ; 0.66 / 0.81 |
| Cone cull of ring triangles | same | same | **0.46** / 0.68 ; 0.63 / 0.79 |
| (diagnostic: no ring walk) | - | - | 0.19 / 0.41 ; 0.27 / 0.42 |
| Exact test of the texel's top triangle as the primary criterion (final) | 99.7% / 2 / 447 / .0005 | 94.1% / 3,098 / 7,146 / .015 | 0.48 / 0.70 ; 0.82 / 0.97 |

The same fixture's hardware Interactive frame (camera rays plus 8 shadow rays, fused) costs 0.27-0.29 ms. The remaining fixture disagreement is at the 15° sun's bump crests: N8 widens some shadow bands by about a texel where the ray grazes a crest that the map samples at a texel centre. The ring walk is the largest single cost (about 0.3 ms of 0.48 here).

S1 also drove three fixes, each found by locating the failing pixels (M):

1. 467/253 umbra leaks in the concave lane: two separate open quads meet at the fold, and a steep open surface crosses the ray below its texel's highest point, between march samples. Fix: confirm candidates with an exact test of the texel's top triangle (and its edge neighbours, for quad diagonals). Leaks fell to 5/3.
2. 33/21 solid flips on lit floor beside the 0.2-5 mm strips: zero-thickness sheets were decided by a texel-quantized crossing test. Fix: sheets are decided only by the exact test. Flips fell to 1/0.
3. Remaining partial error and grazing acne: make the exact test primary for every candidate and drop the "solid below an open surface" assumption (a closed mesh always has a back face below a front face). S1 partial RMS fell from .148 to .083.

## Evidence and time

- **Evidence** (ignored, `E:/renderlab-work/n8/`): `batch/S1` (comparison.csv, scores.json, images), `batch/S2` and `batch/S3` (comparison.json/csv, per-frame.json, timing-frames.json, images), `batch/n8-comparison.json` (the merged S2/S3 tables), `batch/quarry-run/quarry` (scores-traced.json, quarry.json), `batch/quarry-run-b` and `batch/quarry-timing` (the other two quarry timings), `screen-*` and `batch/fixture-run` (fixture screens), and `runs/` (every lease's log, preflight and memory samples). Raw count arrays, crops and references above 5 MB were deleted after scoring.
- **Builds:** development/test profile (renderlab opt-level 1) with the Vulkan validation layer active, as in the recorded Renderlab bench runs; Vulkan/wgpu 30.0.1, NVIDIA 617.14. S1-S3 used build A (this tree before the final comment and test edits). The quarry used builds C/D, which change only comments, the quarry test and the `ring: false` branch (own-triangle skip). Separate target `E:/renderlab-work/n8/target`, -j 2, deleted at the end.
- **Gate:** `cargo test -p renderlab --lib` passed by exit status: 101 passed, 0 failed, 98 ignored.
- **Leases:** 20 short leases under the shared lock with the 4/2 GiB guard, interleaved with N7a's leases. One quarry run tripped the 2 GiB in-run abort during asset upload and was rerun. Two scoring outputs were lost to my own queue script's overly broad cleanup (a stale queue process that TaskStop did not end, and a cleanup line that removed the quarry output) and were rerun: S2 once, the quarry once.
- **Time (UTC):** 07:17-07:40 reading (docs, Unreal and Unity sources); 07:40-08:11 implementation to a first full build; 08:11-08:26 fixture screens and the S1-driven fixes (about 12 one-minute leases); 08:26-09:06 the benches, rescoring and the reference rebuild, about 15 minutes of it GPU leases; 09:06-09:20 this record. The checks were batched per bench and run once on the retained variant, except S1 and the quarry, which needed the fixes and the reference rebuild above.
