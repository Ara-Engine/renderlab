# P-RTS: RT sun-shadow ray cost

2026-11-09, branch `t3/p-rts`, base main `3a71de0` (renderer source identical to ARA1's
`ae85750`). Phase: beat Ara on performance with zero quality loss. This unit owns the
RT path's sun-shadow rays: RTDEF2's adaptive budget, the Interactive shadow kernels and
their TR1 integration.

<!-- P-RTS: results sections are filled from the measured runs below. -->

## 1. What the cut and moving frames spend

ARA1's own per-pass GPU timestamps already isolate the cost (frozen final runs,
1920×1080, RTX 4070 SUPER). `primary_ms` is the fused camera-ray + sun-visibility pass:

| Scene | still (8 rays) | moving (32) | first cut (256) | ms per ray per pixel |
|---|---:|---:|---:|---:|
| game-base | 0.74 | 2.33 | 17.45 | 0.067 |
| ara-pbr-oriented | 0.43 | 1.28 | 9.71 | 0.036 |
| quarry-daylight | 0.54 | 1.30–1.41 | 11.60 | 0.044 |

Primary cost is linear in the ray count with the same camera rays, receivers and
launch: on game-base, (2.33 − 0.74)/24 and (17.45 − 0.74)/248 both give 0.066–0.067 ms
per ray per pixel (about 31 G shadow rays/s). Every other pass is within noise across the
three states, so the cut's +17 ms and the moving +1.6 ms are shadow rays. Per-ray
traversal is already cheap: `FORCE_OPAQUE | TERMINATE_ON_FIRST_HIT`, tmax clipped to
the light-space box, `PREFER_FAST_TRACE` BLAS/TLAS. No traversal tweak can remove a
~10× gap, so the lever has to be the number of rays.

Where the rays go: the sun is a 0.00465 rad disk. Almost every receiver is fully lit or
fully in umbra, so all 256 (or 32) of its rays agree, and its count is exactly 0 or 256
whatever the budget. Only penumbra pixels, a thin band along shadow edges, have counts
that depend on the budget.

## 2. Change: penumbra-listed full budget

When TR1's budget exceeds the held count (cut 256, reset 64, moving 32), the frame runs:

1. **Probe** (the existing fused trace, `primary_trace_interactive_deferred`, and its
   edge repair): every receiver traces the first `held_rays` (8) of its budget's
   directions. That is the same rotated R2 sequence and receiver as before.
2. **List** (`shaders/primary_penumbra_list.wgsl`): a pixel is listed when its 5×5
   window holds both a blocked and a visible probe ray. That covers a mixed pixel, or a
   unanimous-0 pixel beside a unanimous-256 pixel (a sub-pixel edge).
3. **Full budget** (`shaders/primary_penumbra.wgsl`, indirect): each listed pixel
   re-derives its receiver with the probe's own arithmetic (raw camera ray, near-edge
   test, the repair kernel's correction), then traces all budget directions with eight
   lanes per pixel and writes production's count. The estimator body is
   `fresh_estimate`'s own text with a lane stride (`trace_source`), so directions,
   flags, tmax and the integer sum are production's.

Held frames (budget 8) are unchanged: the probe is the budget and no extra pass runs.
Non-listed pixels output the probe's unanimous 0 or 256. That equals production's value
unless some later budget ray would have disagreed. The only possible difference is a
"hidden fringe": a pixel whose own and 24 neighbours' probe rays (200 rays over the
disk) all agree while the budget sees a sliver. The quality section counts these exactly
against the unchanged binary.

Programmable shading is untouched: materials still read one 0..256 visibility per
pixel; no hit record, material input or program ABI changes. Non-RT is unaffected (the
passes only exist in TR1's RT trace module). The legacy Exact and non-reconstructed
Interactive pipelines keep their own estimator.

Patent note: adaptive refinement of sampled soft-shadow visibility where samples
disagree is classic prior art (adaptive supersampling, Whitted 1980; Cook 1986; shadow
ray refinement by Hart et al. 1999). It uses no reservoir, no reuse across pixels or
frames, no denoiser and no variance-guided filter. The shadow-filter claims mapped in
[m7-p0-claims.md](m7-p0-claims.md) §3 (US 11,688,042; US 12,423,782; the history-count
filter-size claim) each recite a spatial filter applied to ray-traced values. Here the
5×5 window only decides which pixels trace more rays: every output is that pixel's own
ray count, unfiltered.
