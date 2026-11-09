# P-RTS: RT sun-shadow ray cost

2026-11-09, branch `t3/p-rts`, merged with main `bdec7b9` (P-GI exact GI cuts, v0.1.0).
Phase: beat Ara on performance with zero quality loss (docs/milestone-8.md). This unit
owns the RT path's sun-shadow rays: RTDEF2's adaptive budget, the Interactive shadow
kernels and their TR1 integration.

**Result.** TR1 cut, reset and moving frames now spend the RTDEF2 budget only near
shadow edges. On the frozen ARA1 scenes (1080p, RTX 4070 SUPER) the game-base first cut
drops from 21.6 to about 5 ms, and the cut's shadow pass from 17.6 to about 1.1–1.4 ms.
Moving frames lose about 1.3–1.6 ms. Held frames are unchanged. Final numbers and the
quality proof are in §4–§5.

## 1. What the cut and moving frames spend

ARA1's own per-pass GPU timestamps isolate the cost (frozen final runs, 1920×1080).
`primary_ms` is the fused camera-ray + sun-visibility pass:

| Scene | still (8 rays) | moving (32) | first cut (256) | ms per ray per pixel |
|---|---:|---:|---:|---:|
| game-base | 0.74 | 2.33 | 17.45 | 0.067 |
| ara-pbr-oriented | 0.43 | 1.28 | 9.71 | 0.036 |
| quarry-daylight | 0.54 | 1.30–1.41 | 11.60 | 0.044 |

The cost is linear in the ray count: same camera rays, receivers and launch, and on
game-base (2.33 − 0.74)/24 and (17.45 − 0.74)/248 both come to 0.066–0.067 ms per ray
per pixel (about 31 G shadow rays/s). The other passes stay within noise across the
three states, so the cut's +17 ms and the moving +1.6 ms are shadow rays. Per-ray
traversal is already lean: `FORCE_OPAQUE | TERMINATE_ON_FIRST_HIT`, tmax clipped to the
light-space box, `PREFER_FAST_TRACE` BLAS/TLAS. A traversal tweak cannot close a ~10×
gap; the ray count has to fall. The planned Nsight GPU Trace was not taken: the GPU queue
was saturated by five units, and the linear per-ray decomposition already answers the
question a trace would.

Where the rays go: the matched scenes' sun is a 0.00465 rad disk. Almost every receiver
is fully lit or fully in umbra, so all of its 256 (or 32) rays agree, and its count is
exactly 0 or 256 whatever the budget. Only penumbra receivers, a thin band at shadow
edges, have budget-dependent counts.

How Ara and Unreal avoid the spike: Ara hybrid's sun shadows are filtered shadow maps
(moment pyramid, contact-hardening kernels; `shaders/look-shadow.slangh` in the
read-only Ara checkout), so a cut costs the same as a still frame. That flat cost comes
with Ara's 57% higher sun-shadow error in ARA1. Unreal's RT shadows trace about one ray
per pixel and denoise. Renderlab keeps its exact per-pixel estimator and stops spending
it where every ray would agree.

## 2. Change: penumbra-listed full budget

On TR1 frames whose budget exceeds 16 (cut 256, reset 64, moving/young 32):

1. **Probe** (the existing fused trace `primary_trace_interactive_deferred` and its edge
   repair): every receiver traces 16 probe rays. Eight are the budget's own first eight
   directions, and eight reuse that sequence over a cone twice the sun's. The probes
   only decide which pixels trace the budget.
2. **List** (`shaders/primary_penumbra_list.wgsl`): a receiver is listed when its 3×3
   window holds both a blocked and a visible probe. That covers a mixed pixel, or
   unanimous-0 and unanimous-256 pixels side by side (a sub-pixel edge).
3. **Budget** (`shaders/primary_penumbra.wgsl`, indirect dispatch, four pixels × eight
   lanes per group): each listed pixel re-derives its receiver with the probe pass's own
   arithmetic (raw camera ray, near-edge test, the repair kernel's correction), then
   traces all budget directions and writes production's count. The estimator body is
   `fresh_estimate`'s own text with a lane stride (`trace_source`), so directions,
   flags, tmax and the integer sum are production's.

A non-listed receiver takes the unanimous 0 or 256. That is production's value unless a
later budget ray would have disagreed: a "hidden fringe" whose whole window saw no
disagreement. §4 counts these against the unchanged main binary.

Held frames (budget 8) are untouched: they keep j and an exact ×1.0 slope, and no
extra pass runs. Programmable shading is untouched too: materials still read one 0..256
visibility per pixel, and no hit record, material input or program ABI changes. Non-RT
is unaffected; the passes exist only in TR1's RT trace module. The legacy Exact and
non-reconstructed Interactive pipelines keep their own estimator. The non-deferred
fallback (edge repair buffer over the device limit) keeps the full budget for every
pixel. Frame words: `PrimaryFrame.sampling.y` holds the probe count, `.z` the in-disk
probes (0 when not listing), `.w` the wide-probe cone scale (f32 bits). New memory:
one 4-byte-per-pixel list buffer (8.3 MB at 1080p, deferred-edge targets only).

Patent note: adaptive refinement of sampled soft-shadow visibility where samples
disagree is classic prior art (adaptive supersampling, Whitted 1980; Cook 1986; lazy
visibility evaluation, Hart et al. 1999). The pass uses no reservoir, no reuse across
pixels or frames, no denoiser and no variance-guided filter. The shadow-filter claims
mapped in [m7-p0-claims.md](m7-p0-claims.md) §3 (US 11,688,042; US 12,423,782; the
history-count filter-size claim) each recite a spatial filter applied to ray-traced
values. Here the window only decides which pixels trace more rays; every output is that
pixel's own ray count, unfiltered.

## 3. Variants screened

Same frozen ARA1 scenes, 960×540 quality captures against the unchanged binary, 1080p
streams for cost. "Diff px" is the most pixels differing in any of the 19 registered
frames, and "codes" the largest 8-bit change. Crop verdicts are ARA1's linear display
RMS against the path-traced references, candidate versus baseline.

| Variant | game-base | quarry | quarry crops worse | game cut / move primary ms | Verdict |
|---|---|---|---:|---|---|
| list every receiver | identical | identical | 0 | 19.6 / 4.0 | exact, slower: proves the budget pass reproduces production bit for bit |
| 8 in-disk probes, 5×5 | ≤3 px, 3 codes | ≤7 px, 1 code | 16 (≤2.3e-5 rel.) | 1.40 / 1.06 | hidden fringes |
| 8 probes, 7×7 / 9×9 | ≤2 px | ≤4 / 3 px | 12 / 11 | 1.74 / 1.29, 2.13 / 1.57 | fewer misses, costlier |
| 16 in-disk probes, 5×5 / 7×7 | ≤1 / 0 px | ≤4 / 3 px | 8 / 8 | 2.4 / 1.6, 2.3 / 1.8 | costlier, still misses |
| listed pixels trace 2× budget | ≤480 px, 6 codes | ≤408 px | 27 | 1.75 / 1.10 | **rejected**: sun-shadow crops worse |
| 8 probes on a 1.5× cone, 5×5 | identical | ≤4 px | 0 | 1.41 / 1.07 | better |
| 8 probes on a 2× cone, 3×3 | identical | 1 px, 1 code | 0 | 1.14 / 0.92 | best ARA1, **fails S2** (below) |
| **8 in-disk + 8 on a 2× cone, 3×3** | §4 | §4 | §4 | §5 | final |

The 2×-cone-only probe failed RTDEF2's S2 fixture, which uses a 0.04 rad light (nine
times the matched sun). There, 280 fringe pixels on the cold frame that production gives
0.934–0.965 came out fully lit, with up to 6.6% visibility error; moving frames erred by
up to 0.16. When a blocker's shadow is no wider than the probe cone, widening spreads
the probes thinly over the real disk. The final design keeps the budget's own eight
in-disk directions for that regime and adds the eight wide probes for thin rim slivers
of a small sun.

<!-- §4–§6 are filled from lease final-2. -->
